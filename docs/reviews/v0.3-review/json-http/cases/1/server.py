"""A test server for #308 area 1: two origins (127.0.0.1:8765 and localhost:8766, the same
socket address family but different hosts), logging every request's headers to log.jsonl.
Routes: /json (a JSON body), /to-other (302 to the other origin), /to-back (302 back to
127.0.0.1:8765/json), /chain (302 to the other origin's /to-back), /big (a large body),
/bad-utf8, /slow (sleeps 5 s), /chunked (a chunked JSON body)."""
import http.server, json, os, sys, threading, time

LOG = os.path.join(os.path.dirname(os.path.abspath(__file__)), "log.jsonl")


class H(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def send(self, status, body=b"", headers=()):
        self.send_response(status)
        for k, v in headers:
            self.send_header(k, v)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        port = self.server.server_address[1]
        with open(LOG, "a") as f:
            f.write(json.dumps({"port": port, "path": self.path, "headers": dict(self.headers.items())}) + "\n")
        p = self.path
        if p == "/json":
            self.send(200, b'{"b":[1,2.50,"\\u00e9"],"a":{"x":null}}', [("Content-Type", "application/json")])
        elif p == "/to-other":
            self.send(302, headers=[("Location", "http://localhost:8766/json")])
        elif p == "/to-back":
            self.send(302, headers=[("Location", "http://127.0.0.1:8765/json")])
        elif p == "/chain":
            self.send(302, headers=[("Location", "http://localhost:8766/to-back")])
        elif p == "/big":
            self.send(200, b"[" + b"1," * (9 * 1024 * 1024) + b"1]")
        elif p == "/bad-utf8":
            self.send(200, b'"\xff"')
        elif p == "/slow":
            time.sleep(5)
            self.send(200, b"null")
        elif p == "/chunked":
            self.send_response(200)
            self.send_header("Transfer-Encoding", "chunked")
            self.end_headers()
            for part in [b'{"a":', b"[1,2", b",3]}"]:
                self.wfile.write(b"%x\r\n%s\r\n" % (len(part), part))
            self.wfile.write(b"0\r\n\r\n")
        else:
            self.send(404, b"not found")

    do_POST = do_GET


for addr in [("127.0.0.1", 8765), ("localhost", 8766)]:
    s = http.server.ThreadingHTTPServer(addr, H)
    threading.Thread(target=s.serve_forever, daemon=True).start()
print("serving", flush=True)
while True:
    time.sleep(3600)
