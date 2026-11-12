//! A copy format: `COPY t TO 'out.quackfile'` and `COPY t FROM 'out.quackfile'`.
//!
//! A quackfile has one line per row and `|` between the columns. Every value
//! is cast to VARCHAR and each of its bytes written as four base-6 digits
//! spelled with [`QUACKS`], so `'Hi'` becomes `QAQQQASC`. A NULL is written
//! as [`NULL_MARKER`].
//!
//! [`CopyToFunctionCallbacks`] is the lifecycle DuckDB runs for `COPY ... TO`:
//!
//! 1. `bind` runs once per statement and inspects the input columns.
//! 2. `init` runs once per output file and opens it.
//! 3. `batch` runs for every batch of input rows, possibly on several workers;
//!    here it encodes the rows.
//! 4. `flush` runs once per batch, in input order; here it writes the encoded rows.
//! 5. `finalize` runs once per output file after the last flush.
//!
//! [`CopyFromFunctionCallbacks`] is the lifecycle for `COPY ... FROM`:
//!
//! 1. `bind` runs once per statement and inspects the target columns.
//! 2. `init_global_state` runs once per read; here it opens the file.
//! 3. `exec` runs until it leaves the output chunk empty.
//!
//! Naming the format after the file extension lets DuckDB pick it from the
//! path alone, without `FORMAT quackfile`.

use std::{num::NonZeroUsize, sync::Mutex};

use duckdb_neo::{
    DuckDBType, Result,
    column_data_collection::ColumnDataCollection,
    connection::Context,
    copy_function::{
        CopyFromBindInfo, CopyFromCardinality, CopyFromFunctionCallbacks, CopyFunctionBuilder,
        CopyToBindInfo, CopyToFunctionCallbacks,
    },
    data_chunk::{DataChunk, DataChunkRef},
    error::Error,
    extension::Extension,
    file::{File, FileBuilder, FileSystem},
    vector::Unknown,
};

/// The format name and file extension.
pub const FORMAT_NAME: &str = "quackfile";

/// The base-6 digits, `Q` is 0 and `S` is 5.
pub const QUACKS: [u8; 6] = *b"QUACKS";

/// The digits one byte takes; `6^4 = 1296` covers every byte.
const DIGITS_PER_BYTE: usize = 4;

/// The field written for NULL; it is not a digit, so it cannot collide with a value.
pub const NULL_MARKER: &str = "-";

pub(crate) const DELIMITER: char = '|';

const READ_SIZE: usize = 64 * 1024;

fn encode(value: &str, out: &mut String) {
    for byte in value.bytes() {
        let mut digits = [0u8; DIGITS_PER_BYTE];
        let mut rest = byte as usize;
        for digit in digits.iter_mut().rev() {
            *digit = QUACKS[rest % QUACKS.len()];
            rest /= QUACKS.len();
        }
        out.extend(digits.map(char::from));
    }
}

fn decode(field: &str) -> std::result::Result<String, String> {
    if !field.len().is_multiple_of(DIGITS_PER_BYTE) {
        return Err(format!(
            "{} characters is not a multiple of {DIGITS_PER_BYTE}",
            field.len()
        ));
    }
    let bytes = field
        .as_bytes()
        .chunks(DIGITS_PER_BYTE)
        .map(|digits| {
            let value = digits.iter().try_fold(0usize, |value, &digit| {
                let digit = QUACKS
                    .iter()
                    .position(|&quack| quack == digit)
                    .ok_or_else(|| format!("'{}' is not one of QUACKS", char::from(digit)))?;
                Ok::<_, String>(value * QUACKS.len() + digit)
            })?;
            u8::try_from(value).map_err(|_| format!("{value} does not fit in a byte"))
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    String::from_utf8(bytes).map_err(|_| "the bytes are not valid UTF-8".to_string())
}

/// The registered implementation of both directions.
struct QuackFile;

fn reject_options(count: usize) -> Result<()> {
    if count > 0 {
        return Err(Error::invalid_input(format!(
            "{FORMAT_NAME} does not take any options"
        )));
    }
    Ok(())
}

impl CopyToFunctionCallbacks for QuackFile {
    /// Whatever `bind` computes once per statement; every column is cast to VARCHAR, so nothing.
    type BindData = ();
    /// One per output file; a `Mutex` because the file is written from `flush`.
    type InitData = Mutex<File>;
    /// The encoded lines of one batch, handed from `batch` to `flush`.
    type BatchData = String;

    /// Runs once per statement. Typical work:
    /// - Check the input columns and the format options, returning an error for what
    ///   the format cannot write.
    fn bind(&self, _context: &Context, bind_info: &CopyToBindInfo) -> Result<Self::BindData> {
        reject_options(bind_info.option_count()?)
    }

    /// Runs once per output file. `file_path` may differ from the statement's path,
    /// e.g. for temporary files or `PARTITION_BY`.
    fn init(
        &self,
        context: &Context,
        _bind_data: &Self::BindData,
        file_path: &str,
    ) -> Result<Self::InitData> {
        let fs = FileSystem::new(context)?;
        let file = FileBuilder::new(&fs, file_path)?
            .write()
            .create_new()
            .open()?;
        Ok(Mutex::new(file))
    }

    /// Runs for every batch, possibly on several workers at once, so the expensive
    /// encoding happens here rather than in `flush`.
    fn batch(
        &self,
        context: &Context,
        _bind_data: &Self::BindData,
        _init_data: &Self::InitData,
        input: &ColumnDataCollection<'_>,
    ) -> Result<Self::BatchData> {
        let mut encoded = String::new();
        let mut scratch = DataChunk::create(context, [String::logical_type(context)?], true)?;
        let mut scan = input.to_scan(context)?;

        while let Some(chunk) = scan.next_chunk()? {
            let mut lines = vec![String::new(); chunk.row_count()?];

            // Column by column, since each cast reuses the one scratch vector.
            for column in 0..chunk.col_count() {
                let source = chunk.get_vector_at::<Unknown>(column)?;
                let text = scratch
                    .get_vector_at_mut::<String>(0)?
                    .cast_rows(source, context)?;

                for (line, value) in lines.iter_mut().zip(text.reader()?.iter()) {
                    if column > 0 {
                        line.push(DELIMITER);
                    }
                    match value {
                        Some(value) => encode(value, line),
                        None => line.push_str(NULL_MARKER),
                    }
                }
            }

            for line in lines {
                encoded.push_str(&line);
                encoded.push('\n');
            }
        }
        Ok(encoded)
    }

    /// Runs once per batch, in input order, so rows keep their order in the file.
    fn flush(
        &self,
        _context: &Context,
        _bind_data: &Self::BindData,
        init_data: &Self::InitData,
        _input: &ColumnDataCollection<'_>,
        batch_data: &mut Self::BatchData,
    ) -> Result<()> {
        init_data.lock().unwrap().write(batch_data.as_bytes())?;
        Ok(())
    }

    fn finalize(
        &self,
        _context: &Context,
        _bind_data: &Self::BindData,
        init_data: &Self::InitData,
    ) -> Result<()> {
        init_data.lock().unwrap().close()
    }
}

/// What `COPY ... FROM` needs after binding.
struct QuackReadBind {
    path: String,
    columns: usize,
}

/// Reads a file one line at a time; shared by the read's single worker.
pub(crate) struct QuackReader {
    file: File,
    pending: Vec<u8>,
    at_end: bool,
    line_number: usize,
}

impl QuackReader {
    pub(crate) fn open(context: &Context, path: &str) -> Result<Self> {
        let fs = FileSystem::new(context)?;
        let file = FileBuilder::new(&fs, path)?.read().open()?;
        Ok(QuackReader {
            file,
            pending: Vec::new(),
            at_end: false,
            line_number: 0,
        })
    }

    pub(crate) fn next_line(&mut self) -> Result<Option<String>> {
        loop {
            if let Some(end) = self.pending.iter().position(|&byte| byte == b'\n') {
                let line: Vec<u8> = self.pending.drain(..=end).take(end).collect();
                return self.finish_line(line).map(Some);
            }
            if self.at_end {
                // A last line without its newline still counts.
                if self.pending.is_empty() {
                    return Ok(None);
                }
                let line = std::mem::take(&mut self.pending);
                return self.finish_line(line).map(Some);
            }
            let read = self.file.read(READ_SIZE)?;
            self.at_end = read.is_empty();
            self.pending.extend(read);
        }
    }

    fn finish_line(&mut self, line: Vec<u8>) -> Result<String> {
        self.line_number += 1;
        String::from_utf8(line).map_err(|_| self.error("the line is not valid UTF-8"))
    }

    fn error(&self, message: impl std::fmt::Display) -> Error {
        Error::invalid_input(format!(
            "{FORMAT_NAME} line {}: {message}",
            self.line_number
        ))
    }

    /// Read and decode up to `limit` rows of `columns` fields each; fewer only at the end of the file.
    pub(crate) fn next_rows(
        &mut self,
        limit: usize,
        columns: usize,
    ) -> Result<Vec<Vec<Option<String>>>> {
        let mut rows = Vec::new();
        while rows.len() < limit
            && let Some(line) = self.next_line()?
        {
            rows.push(self.parse_line(&line, columns)?);
        }
        Ok(rows)
    }

    fn parse_line(&self, line: &str, columns: usize) -> Result<Vec<Option<String>>> {
        let fields: Vec<&str> = line.split(DELIMITER).collect();
        if fields.len() != columns {
            return Err(self.error(format!("expected {columns} fields, found {}", fields.len())));
        }
        fields
            .into_iter()
            .map(|field| match field {
                NULL_MARKER => Ok(None),
                field => decode(field)
                    .map(Some)
                    .map_err(|message| self.error(message)),
            })
            .collect()
    }
}

impl CopyFromFunctionCallbacks for QuackFile {
    /// Whatever `bind` computes once per statement; here the path and the column count.
    type BindData = QuackReadBind;
    /// Shared by every worker of one read; here the open file.
    type GlobalState = Mutex<QuackReader>;
    /// Owned by one worker; the single reader needs nothing of its own.
    type LocalState = ();

    /// Runs once per statement. Typical work:
    /// - Check the target columns and options, returning an error for what the format
    ///   cannot read.
    /// - Return a cardinality estimate when one is cheap to get; here there is none.
    fn bind(
        &self,
        _context: &Context,
        file_path: &str,
        bind_info: &CopyFromBindInfo,
    ) -> Result<(Self::BindData, Option<CopyFromCardinality>)> {
        reject_options(bind_info.option_count()?)?;
        let bind_data = QuackReadBind {
            path: file_path.to_string(),
            columns: bind_info.column_count()?,
        };
        Ok((bind_data, None))
    }

    /// Runs once per read. Lines must be read in order, so the read is limited to one thread.
    fn init_global_state(
        &self,
        bind_data: &Self::BindData,
        context: &Context,
    ) -> Result<(Option<Self::GlobalState>, Option<NonZeroUsize>)> {
        let reader = QuackReader::open(context, &bind_data.path)?;
        Ok((Some(Mutex::new(reader)), NonZeroUsize::new(1)))
    }

    /// Runs until the output chunk stays empty. Typical work:
    /// - Fill each output column, which already has the target table's type.
    /// - Set the size of the first vector to the number of rows; 0 ends the read.
    ///
    /// Here each column is decoded to VARCHAR and then cast to the target type.
    fn exec(
        &self,
        bind_data: &Self::BindData,
        global_state: Option<&Self::GlobalState>,
        _local_state: Option<&mut Self::LocalState>,
        context: &Context,
        mut output: DataChunkRef<'_>,
    ) -> Result<()> {
        let rows = global_state
            .ok_or_else(|| Error::internal("quackfile: global state missing"))?
            .lock()
            .unwrap()
            .next_rows(output.capacity()?, bind_data.columns)?;

        for column in 0..bind_data.columns {
            // Fresh per column: casting to a VARCHAR column shares the scratch buffer instead of copying it.
            let mut scratch = DataChunk::create(context, [String::logical_type(context)?], true)?;
            let mut text = scratch.get_vector_at_mut::<String>(0)?;
            text.set_size(rows.len())?;
            let mut writer = text.writer()?;
            for (row, fields) in rows.iter().enumerate() {
                writer.set(row, fields[column].as_deref())?;
            }
            drop(writer);

            output
                .get_vector_at_mut::<Unknown>(column)?
                .cast_rows(&text, context)?;
        }
        Ok(())
    }
}

pub(crate) fn register(extension: &Extension) -> Result<()> {
    CopyFunctionBuilder::new(FORMAT_NAME, QuackFile)
        .with_fallback_extension(FORMAT_NAME)
        .register_to_and_from(extension)
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};

    #[test]
    fn round_trips_bytes() {
        let mut encoded = String::new();
        encode("Hi 🦆", &mut encoded);
        assert_eq!(&encoded[..8], "QAQQQASC");
        assert_eq!(decode(&encoded).unwrap(), "Hi 🦆");
    }

    #[test]
    fn rejects_malformed_fields() {
        assert!(decode("QAQ").is_err());
        assert!(decode("QAQX").is_err());
        assert!(decode("SSSS").is_err());
    }
}
