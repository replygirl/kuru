"""Cancel real in-flight HTTP work, keep a draft, and complete another turn."""

import fcntl
import http.server
import json
import os
import pty
import struct
import subprocess
import sys
import termios
import threading
import time
from pathlib import Path

from terminal_driver import Terminal

started = threading.Event()
release = threading.Event()


class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def reply(self, value):
        body = json.dumps(value).encode()
        try:
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass  # Cancellation is expected to close pending connections.

    def do_GET(self):
        self.reply({"data": [{"id": "fixture"}]})

    def do_POST(self):
        self.rfile.read(int(self.headers["Content-Length"]))
        delayed = not release.is_set()
        started.set()
        release.wait(timeout=15)
        self.reply(
            {
                "status": "completed",
                "output": [
                    {
                        "type": "message",
                        "content": [
                            {
                                "type": "output_text",
                                "text": "LATE_RESPONSE_MUST_STAY_ABSENT"
                                if delayed
                                else "FRESH_RESPONSE_MARKER",
                            }
                        ],
                    }
                ],
            }
        )


server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Provider)
threading.Thread(target=server.serve_forever, daemon=True).start()
config = Path(sys.argv[3]).parent / "provider.toml"
config.write_text(
    f"api_base='http://127.0.0.1:{server.server_port}/v1'\n"
    "api_key_env='KURU_FIXTURE_KEY'\nmax_rounds=1\n"
)
master, slave = pty.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 35, 120, 0, 0))
before = termios.tcgetattr(slave)
environment = os.environ.copy()
environment.update(
    TERM="xterm-256color", KURU_REDUCED_MOTION="1", KURU_FIXTURE_KEY="fixture"
)
child = subprocess.Popen(
    [
        sys.argv[1],
        "-C",
        sys.argv[2],
        "--data-dir",
        sys.argv[3],
        "--provider",
        "responses",
        "--model",
        "fixture",
        "--mode",
        "freudian",
        "--config",
        str(config),
        "--no-dream",
    ],
    stdin=slave,
    stdout=slave,
    stderr=slave,
    env=environment,
)
terminal = Terminal(master, child)
output = terminal.output
read_for = terminal.read_for
screen_text = terminal.screen_text
wait_text = terminal.wait_text


try:
    wait_text(b"KURU")
    os.write(master, b"Slow request\r")
    # Keep draining the PTY while waiting for the provider. A partial first
    # frame can contain KURU before it fills the PTY buffer; blocking here can
    # otherwise prevent the app from finishing its draw and consuming input.
    deadline = time.monotonic() + 10
    while not started.is_set():
        read_for(0.05)
        assert child.poll() is None, bytes(output[-6000:])
        assert time.monotonic() < deadline, screen_text().decode()
    os.write(master, b"\x1b[200~Next thought\x1b[201~")
    os.write(master, b"\x1bOQ")  # F2 explains why settings are temporarily busy.
    wait_text(b"current turn")
    os.write(master, b"\x1b")
    wait_text(b"Cancelled")
    release.set()
    read_for(0.2)
    os.write(master, b"\r")  # The preserved draft becomes the next real turn.
    wait_text(b"FRESH_RESPONSE_MARKER")
    assert b"LATE_RESPONSE_MUST_STAY_ABSENT" not in output
    terminal.wait_idle()
    os.write(master, b"/quit\r")
    terminal.wait_exit()
    assert termios.tcgetattr(slave) == before
finally:
    release.set()
    if child.poll() is None:
        child.kill()
        child.wait()
    server.shutdown()
    server.server_close()
    os.close(master)
    os.close(slave)
