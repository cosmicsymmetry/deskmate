"""Check the tunnel Caddyfile in docs/self-host.md using an installed Caddy binary."""

import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import re
import socket
import subprocess
import sys
import tempfile
import threading
import time


class Echo(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(self.headers.get("X-Forwarded-For", "").encode())

    def log_message(self, *_args):
        pass


def request(port, headers):
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=2)
    try:
        connection.request("GET", "/", headers={"Host": "desk.example.org", **headers})
        response = connection.getresponse()
        assert response.status == 200, response.status
        return response.read().decode()
    finally:
        connection.close()


def check(caddy, recipe, backend_port, trusted):
    with socket.socket() as reserve:
        reserve.bind(("127.0.0.1", 0))
        port = reserve.getsockname()[1]
    config = recipe.replace(":8080", f":{port}").replace(":8443", f":{backend_port}")
    config = config.replace("{\n", "{\n    admin off\n    persist_config off\n", 1)
    if not trusted:
        # The same TCP client is now outside Caddy's trusted proxy list.
        config = config.replace("127.0.0.1/32 ::1/128", "192.0.2.10/32")
    with tempfile.TemporaryDirectory(prefix="deskmate-proxy-") as directory:
        path = Path(directory) / "Caddyfile"
        path.write_text(config)
        with (Path(directory) / "caddy.log").open("w+") as log:
            process = subprocess.Popen([caddy, "run", "--config", str(path)], stdout=log, stderr=log)
            try:
                deadline = time.monotonic() + 10
                while True:
                    try:
                        request(port, {})
                        break
                    except OSError:
                        if process.poll() is not None or time.monotonic() >= deadline:
                            log.seek(0)
                            raise AssertionError(log.read())
                        time.sleep(0.02)
                for visitor in ["198.51.100.7", "203.0.113.9", "2001:db8::7"]:
                    forwarded = request(port, {
                        "CF-Connecting-IP": visitor,
                        "X-Forwarded-For": "192.0.2.99, 192.0.2.100",
                    })
                    assert forwarded == (visitor if trusted else "127.0.0.1"), forwarded
                for headers in [
                    {"X-Forwarded-For": "192.0.2.99"},
                    {"CF-Connecting-IP": "not-an-ip", "X-Forwarded-For": "192.0.2.99"},
                ]:
                    assert request(port, headers) == "127.0.0.1"
            finally:
                process.terminate()
                process.wait(timeout=5)


def main():
    docs = Path(__file__).resolve().parents[2] / "docs/self-host.md"
    recipe = re.search(r"<!-- tunnel-caddyfile -->\n```caddyfile\n(.*?)```", docs.read_text(), re.S)[1]
    backend = ThreadingHTTPServer(("127.0.0.1", 0), Echo)
    thread = threading.Thread(target=backend.serve_forever, daemon=True)
    thread.start()
    try:
        for trusted in [True, False]:
            check(sys.argv[1], recipe, backend.server_port, trusted)
    finally:
        backend.shutdown()
        backend.server_close()
        thread.join()
    print("PASS: distinct IPv4/IPv6 clients; spoofed, missing and malformed headers; untrusted peer")


if __name__ == "__main__":
    main()
