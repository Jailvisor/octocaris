#!/usr/bin/env python3
"""Trickle HTTP server used to verify download cancellation."""
import socketserver
import sys
import time


class Handler(socketserver.BaseRequestHandler):
    def handle(self):
        self.request.recv(4096)
        self.request.sendall(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/zip\r\n"
            b"Content-Length: 1000000\r\nConnection: close\r\n\r\n"
        )
        chunk = b"x" * 1024
        try:
            for _ in range(1000):
                self.request.sendall(chunk)
                time.sleep(0.05)
        except OSError:
            pass


class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8766
    with Server(("127.0.0.1", port), Handler) as server:
        server.serve_forever()
