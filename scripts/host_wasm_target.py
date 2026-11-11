from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import pathlib
import re


class Handler(SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header("Access-Control-Allow-Origin", "https://shell.duckdb.org")
        self.send_header("Access-Control-Allow-Methods", "GET, HEAD, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "Range")
        self.send_header("Access-Control-Expose-Headers", "Content-Length, Content-Range, Accept-Ranges")
        self.send_header("Access-Control-Allow-Private-Network", "true")
        super().end_headers()

    def do_OPTIONS(self):
        self.send_response(204)
        self.send_header("Content-Length", "0")
        self.end_headers()


cwd = pathlib.Path(__file__).parent.resolve()

handler = partial(Handler, directory=f"{cwd.parent}/build/wasm_eh/release/")

makefile = (cwd.parent / "Makefile").read_text()
extension_name = re.search(
    r"^\s*EXTENSION_NAME\s*(?::=|\?=|\+=|=)\s*([^\s#]+)",
    makefile,
    re.MULTILINE,
).group(1)



print("Serving wasm_eh build at http://127.0.0.1:8000")
print("Open https://shell.duckdb.org/?allowUnsignedExtensions=true and load the extension:")
print("```")
print(f"INSTALL AND LOAD 'http://127.0.0.1:8000/{extension_name}.duckdb_extension.wasm';")
print("```\n")
print("Press Ctrl+C to stop the server")
ThreadingHTTPServer(("127.0.0.1", 8000), handler).serve_forever()
