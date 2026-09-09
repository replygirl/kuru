"""Exercise selections through real terminal events and fresh executable launches."""

import fcntl
import os
import pty
import sqlite3
import struct
import subprocess
import sys
import termios
import time

import tomllib
from terminal_driver import Terminal

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
    terminal = Terminal(master, child, rows=38, columns=130)
    output = terminal.output
    read_for = terminal.read_for

    try:
        terminal.wait(
            lambda: all(
                value.encode() in terminal.screen_text().lower()
                for value in [*expected, "enter send"]
            ),
            f"initial selections {expected!r}",
        )
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
            terminal.wait_idle()
            if keys.startswith(b"/"):
                terminal.command(keys.decode().rstrip("\r"))
            else:
                os.write(master, keys[:3])  # F2/F3/F4 opens a real selector.
                terminal.wait_text(b"Esc back")
                terminal.close_picker(keys[3:])
            deadline = time.monotonic() + 10
            while True:
                read_for(0.05)
                actual = configuration()
                # Assert the current rendered error, not historical output.
                visible = terminal.screen_text()
                error_visible = expected_error is None or all(
                    word in visible for word in expected_error.split()
                )
                if (
                    b"enter send" in visible
                    and error_visible
                    and all(actual.get(key) == value for key, value in choices.items())
                ):
                    break
                assert child.poll() is None, bytes(output)
                assert time.monotonic() < deadline, (actual, bytes(output[-10000:]))
        os.write(master, b"/quit\r")
        terminal.wait_exit()
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
