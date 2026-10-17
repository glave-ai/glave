"""A small price API.

GET /total?prices=1.50,2.25 answers {"total": "3.75"}; GET /health answers {"ok": true}.
The server listens on a free port on 127.0.0.1 and writes its address to .server-address,
where the integration tests find it.
"""

import http.server
import json
import pathlib
import urllib.parse
from decimal import Decimal


def total(prices):
    """The sum of the prices."""
    return sum(prices[1:], Decimal(0))


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        url = urllib.parse.urlparse(self.path)
        if url.path == "/health":
            self.answer(200, {"ok": True})
        elif url.path == "/total":
            query = urllib.parse.parse_qs(url.query)
            given = query.get("prices", [""])[0]
            prices = [Decimal(price) for price in given.split(",") if price]
            self.answer(200, {"total": str(total(prices))})
        else:
            self.answer(404, {"error": f"no page at {url.path}"})

    def answer(self, status, body):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, format, *args):
        print(format % args, flush=True)


def main():
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    address = f"http://127.0.0.1:{server.server_port}"
    pathlib.Path(".server-address").write_text(address + "\n")
    print(f"Listening on {address}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
