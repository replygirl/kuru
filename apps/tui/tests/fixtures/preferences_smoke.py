"""Exercise selections through real terminal events and fresh executable launches."""

import fcntl
import os
import pty
import re
import select
import sqlite3
import struct
import subprocess
import sys
import termios
import time

import tomllib

BASE = [
    sys.argv[1],
    "-C",
    sys.argv[2],
    "--data-dir",
    sys.argv[3],
    "--provider",
    "demo",
    "--no-dream",
]


def configuration():
    return tomllib.loads(subprocess.check_output([*BASE, "config"], text=True))


def session(expected, actions, expected_error=None):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 38, 130, 0, 0))
    before = termios.tcgetattr(slave)
    environment = os.environ.copy()
    environment.update(TERM="xterm-256color", KURU_REDUCED_MOTION="1")
    child = subprocess.Popen(
        BASE, stdin=slave, stdout=slave, stderr=slave, env=environment
    )
    output = bytearray()

    def read_for(seconds):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if select.select([master], [], [], 0.02)[0]:
                try:
                    output.extend(os.read(master, 65536))
                except OSError:
                    break

    try:
        deadline = time.monotonic() + 10
        while True:
            read_for(0.05)
            assert child.poll() is None, bytes(output)
            visible = re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", output).lower()
            if all(value.encode() in visible for value in expected):
                break
            assert time.monotonic() < deadline, bytes(output[-10000:])
        if expected_error is not None:
            with sqlite3.connect(
                os.path.join(sys.argv[3], "memory.sqlite3")
            ) as connection:
                connection.execute("""
                    CREATE TRIGGER reject_mode_update BEFORE UPDATE ON state
                    WHEN NEW.key LIKE '%/sessions'
                    BEGIN SELECT RAISE(ABORT, 'preference write rejected'); END
                """)
        for keys, choices in actions:
            os.write(master, keys)
            deadline = time.monotonic() + 10
            while True:
                read_for(0.05)
                actual = configuration()
                # Cursor-addressed terminal diffs can omit spaces that already
                # occupy the right cells; each word must still be rendered.
                visible = re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", output)
                error_visible = expected_error is None or all(
                    word in visible for word in expected_error.split()
                )
                if error_visible and all(
                    actual.get(key) == value for key, value in choices.items()
                ):
                    break
                assert child.poll() is None, bytes(output)
                assert time.monotonic() < deadline, (actual, bytes(output[-10000:]))
        os.write(master, b"/quit\r")
        read_for(0.4)
        child.wait(timeout=5)
        assert child.returncode == 0, bytes(output[-10000:])
        assert termios.tcgetattr(slave) == before
    finally:
        if expected_error is not None:
            with sqlite3.connect(
                os.path.join(sys.argv[3], "memory.sqlite3")
            ) as connection:
                connection.execute("DROP TRIGGER IF EXISTS reject_mode_update")
        if child.poll() is None:
            child.kill()
            child.wait()
        os.close(master)
        os.close(slave)


session(
    ["demo", "ifs"],
    [
        (b"/mode jungian\r", {"mode": "jungian"}),
        (b"/model persistent-demo\r", {"model": "persistent-demo"}),
        (b"/effort high\r", {"effort": "high"}),
    ],
)
assert configuration()["mode"] == "jungian"
session(
    ["persistent-demo", "jungian", "high"],
    [
        (b"\x1bOS\x1b[A\r", {"mode": "freudian"}),  # F4: previous mode
        (b"\x1bOQ\r", {"model": "demo", "effort": None}),  # F2: demo
        (b"/effort high\r", {"effort": "high"}),
        (b"\x1bOR\r", {"effort": None}),  # F3: provider default
    ],
)
session(["demo", "freudian", "default"], [])
assert configuration()["mode"] == "freudian"
assert configuration()["model"] == "demo"
assert "effort" not in configuration()

# Reject the final write in the preference/topology/session transaction. Earlier
# writes must roll back too; the terminal must show the failure and retain its
# original mode. This exercises a real SQLite error through the actual UI.
session(
    ["demo", "freudian", "default"],
    [(b"/mode jungian\r", {"mode": "freudian", "model": "demo"})],
    expected_error=b"preference write rejected",
)
assert configuration()["mode"] == "freudian"
