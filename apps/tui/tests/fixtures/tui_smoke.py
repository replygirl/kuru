import fcntl
import os
import pty
import select
import struct
import subprocess
import sys
import termios
import time

master, slave = pty.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 35, 120, 0, 0))
before = termios.tcgetattr(slave)
child = subprocess.Popen(
    [
        sys.argv[1],
        "-C",
        sys.argv[2],
        "--data-dir",
        sys.argv[3],
        "--provider",
        "demo",
        "--mode",
        "freudian",
        "--no-dream",
    ],
    stdin=slave,
    stdout=slave,
    stderr=slave,
)
output = bytearray()


def read_for(seconds):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if select.select([master], [], [], 0.03)[0]:
            try:
                output.extend(os.read(master, 65536))
            except OSError:
                break


try:
    read_for(0.6)
    assert child.poll() is None, bytes(output)
    assert b"KURU" in output, bytes(output)
    for data in [
        b"/help\r",
        b"hello from a terminal\r",
        b"/parts\r",
        b"\x1bOQ",
        b"\r",
        b"/effort default\r",
        b"/mode jungian\r",
        b"/model\r",
        b"\x1b",
        b"/effort\r",
        b"\r",
        b"/mode\r",
        b"\x1b[B",
        b"\r",
        b"/dream\r",
        b"/unknown\r",
    ]:
        os.write(master, data)
        read_for(0.15)
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 20, 65, 0, 0))
    os.write(master, b"\x1b[200~pasted text\x1b[201~")
    read_for(0.1)
    os.write(master, b"\x01")
    os.write(master, b"\r")
    read_for(0.2)
    os.write(master, b"/quit\r")
    read_for(0.5)
    child.wait(timeout=5)
    assert child.returncode == 0, bytes(output[-12000:])
    # Ratatui emits cursor-addressed diffs, so typed characters need not be
    # contiguous in the byte stream. The durable transcript proves submission.
    assert b"demo" in output
    assert termios.tcgetattr(slave) == before, "terminal attributes were not restored"
finally:
    if child.poll() is None:
        child.kill()
        child.wait()
    os.close(master)
    os.close(slave)
