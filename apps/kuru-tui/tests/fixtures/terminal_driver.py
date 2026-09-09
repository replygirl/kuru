"""Bounded observation of real PTY output, including cursor-addressed diffs."""

import os
import re
import select
import time
import unicodedata


class Terminal:
    def __init__(self, master, child, rows=35, columns=120):
        self.master = master
        self.child = child
        self.rows = rows
        self.columns = columns
        self.output = bytearray()

    def read_for(self, seconds):
        deadline = time.monotonic() + seconds
        while (remaining := deadline - time.monotonic()) > 0:
            if select.select([self.master], [], [], min(0.02, remaining))[0]:
                try:
                    data = os.read(self.master, 65536)
                except OSError:
                    if self.child.poll() is None:
                        raise
                    break
                if not data:
                    break
                self.output.extend(data)

    def screen_text(self):
        # Ratatui omits unchanged cells, so ANSI stripping can both lose text and
        # match stale output. Replay the backend's cursor/erase sequences.
        cells = [[" "] * self.columns for _ in range(self.rows)]
        x = y = 0
        for token in re.split(
            r"(\x1b\[[0-?]*[ -/]*[@-~])", self.output.decode(errors="replace")
        ):
            if token.startswith("\x1b["):
                command, parameters = token[-1], token[2:-1]
                if command in ("H", "f"):
                    values = parameters.split(";")
                    y = int(values[0] or "1") - 1
                    x = int(values[1] or "1") - 1 if len(values) > 1 else 0
                elif command == "J" and parameters == "2":
                    cells = [[" "] * self.columns for _ in range(self.rows)]
                elif command == "K" and 0 <= y < self.rows:
                    cells[y][max(x, 0) :] = [" "] * max(self.columns - max(x, 0), 0)
                continue
            for char in token:
                if char == "\r":
                    x = 0
                elif char == "\n":
                    y += 1
                elif ord(char) >= 32 and not unicodedata.combining(char):
                    if 0 <= y < self.rows and 0 <= x < self.columns:
                        cells[y][x] = char
                    x += 2 if unicodedata.east_asian_width(char) in ("W", "F") else 1
        return "\n".join("".join(row) for row in cells).encode()

    def wait(self, predicate, description, timeout=10):
        deadline = time.monotonic() + timeout
        while True:
            self.read_for(0.02)
            if predicate():
                return
            assert self.child.poll() is None, (
                description,
                self.child.returncode,
                bytes(self.output[-6000:]),
            )
            assert time.monotonic() < deadline, (
                description,
                self.screen_text().decode(),
            )

    def wait_text(self, *markers, absent=()):
        def matches():
            visible = self.screen_text()
            return all(m in visible for m in markers) and not any(
                m in visible for m in absent
            )

        self.wait(matches, f"screen contains {markers!r}, excludes {absent!r}")

    def wait_idle(self):
        self.wait_text(b"enter send")

    def command(self, text, picker=None):
        self.wait_idle()
        os.write(self.master, text.encode())
        # Observe the draft before submitting: an old idle frame must never
        # satisfy the completion wait for a command that hasn't been read yet.
        self.wait(
            lambda: text.encode() in b"\n".join(self.screen_text().splitlines()[-5:]),
            f"draft contains {text!r}",
        )
        os.write(self.master, b"\r")
        if picker is not None:
            self.wait_text(picker, b"Esc back")
        else:
            self.wait_text(b"What shall we explore or build?", b"enter send")

    def close_picker(self, keys):
        os.write(self.master, keys)
        self.wait_text(b"enter send", absent=(b"Esc back",))

    def wait_exit(self, timeout=5):
        # Continue draining until the process exits. Waiting on the child with
        # an unread PTY can deadlock on the final redraw or terminal cleanup.
        self.wait(lambda: self.child.poll() is not None, "process exits", timeout)
        self.read_for(0.02)
        assert self.child.returncode == 0, bytes(self.output[-6000:])
