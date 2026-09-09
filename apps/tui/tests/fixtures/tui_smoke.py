import fcntl
import os
import pty
import re
import select
import struct
import subprocess
import sys
import termios
import time


def run_smoke(reduced_motion, full_session):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 35, 120, 0, 0))
    before = termios.tcgetattr(slave)
    environment = os.environ.copy()
    environment.pop("NO_COLOR", None)
    environment["TERM"] = "xterm-256color"
    environment["COLORTERM"] = "truecolor"
    environment.pop("KURU_REDUCED_MOTION", None)
    if reduced_motion:
        environment["KURU_REDUCED_MOTION"] = "1"
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
        env=environment,
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
        # Wait for readiness, not a fixed startup delay: coverage and regular
        # tests build/run concurrently, and process scheduling can vary in CI.
        deadline = time.monotonic() + 10
        while True:
            read_for(0.05)
            assert child.poll() is None, bytes(output)
            visible_text = re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", output)
            if b"KURU" in visible_text:
                break
            assert time.monotonic() < deadline, "terminal did not become ready"
        read_for(0.2)
        assert b"\x1b[38;2;" in output, "truecolor terminal received no RGB colors"
        # Ambient decoration stays alive after the former four-second cutoff.
        # The startup accessibility override must leave every cell settled.
        read_for(4.1)
        settled = len(output)
        read_for(0.7)
        assert (len(output) == settled) == reduced_motion, bytes(output[-4000:])
        # Losing focus pauses ambient work, while the draft remains editable.
        os.write(master, b"\x1b[O")
        read_for(0.2)
        settled = len(output)
        read_for(0.4)
        assert len(output) == settled, "unfocused terminal is still animating"
        os.write(master, b"\x1b[I")
        read_for(0.2)
        if full_session:
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
        assert termios.tcgetattr(slave) == before, (
            "terminal attributes were not restored"
        )
    finally:
        if child.poll() is None:
            child.kill()
            child.wait()
        os.close(master)
        os.close(slave)


run_smoke(reduced_motion=False, full_session=True)
run_smoke(reduced_motion=True, full_session=False)
