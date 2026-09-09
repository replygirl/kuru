"""Cancel real in-flight HTTP work, keep a draft, and complete another turn."""

import fcntl
import http.server
import json
import os
import pty
import re
import select
import struct
import subprocess
import sys
import termios
import threading
import time
import unicodedata
from pathlib import Path

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
output = bytearray()


def read_for(seconds):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if select.select([master], [], [], 0.02)[0]:
            output.extend(os.read(master, 65536))


def screen_text():
    # Replay the cursor-addressed subset emitted by Ratatui. Stripping ANSI is
    # insufficient: unchanged letters and spaces are omitted from each diff.
    cells = [[" "] * 120 for _ in range(35)]
    x = y = 0
    for token in re.split(
        r"(\x1b\[[0-?]*[ -/]*[@-~])", output.decode(errors="replace")
    ):
        if token.startswith("\x1b["):
            command, parameters = token[-1], token[2:-1]
            if command in ("H", "f"):
                values = parameters.split(";")
                y = int(values[0] or "1") - 1
                x = int(values[1] or "1") - 1 if len(values) > 1 else 0
            elif command == "J" and parameters == "2":
                cells = [[" "] * 120 for _ in range(35)]
            elif command == "K" and 0 <= y < 35:
                cells[y][max(x, 0) :] = [" "] * (120 - max(x, 0))
            continue
        for char in token:
            if char == "\r":
                x = 0
            elif char == "\n":
                y += 1
            elif ord(char) >= 32 and not unicodedata.combining(char):
                if 0 <= y < 35 and 0 <= x < 120:
                    cells[y][x] = char
                x += 2 if unicodedata.east_asian_width(char) in ("W", "F") else 1
    return "\n".join("".join(row) for row in cells).encode()


def wait_text(marker):
    deadline = time.monotonic() + 10
    while True:
        read_for(0.05)
        visible = screen_text()
        if marker in visible:
            return
        assert child.poll() is None, bytes(output[-6000:])
        assert time.monotonic() < deadline, (marker, visible.decode())


try:
    wait_text(b"KURU")
    os.write(master, b"Slow request\r")
    assert started.wait(timeout=10), bytes(output[-6000:])
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
    os.write(master, b"/quit\r")
    read_for(0.4)
    child.wait(timeout=5)
    assert child.returncode == 0, bytes(output[-6000:])
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
