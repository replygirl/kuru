import fcntl
import os
import pty
import signal
import struct
import subprocess
import sys
import termios
import threading

from terminal_driver import Terminal


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
    terminal = Terminal(master, child)
    output = terminal.output
    read_for = terminal.read_for
    resume = None

    def pause_child():
        nonlocal resume
        os.kill(child.pid, signal.SIGSTOP)
        resume = threading.Timer(1.0, os.kill, args=(child.pid, signal.SIGCONT))
        resume.start()

    try:
        terminal.wait_text(b"KURU", b"enter send")
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
            terminal.command("/help")
            pause_child()  # A real scheduling delay longer than the old 150 ms.
            terminal.command("hello from a terminal")
            resume.join()
            terminal.command("/parts")
            os.write(master, b"\x1bOQ")
            terminal.wait_text(b"Models", b"Esc back")
            terminal.close_picker(b"\r")
            terminal.command("/effort default")
            terminal.command("/mode jungian")
            terminal.command("/model", picker=b"Models")
            terminal.close_picker(b"\x1b")
            terminal.command("/effort", picker=b"Efforts")
            terminal.close_picker(b"\r")
            terminal.command("/mode", picker=b"Modes")
            terminal.close_picker(b"\x1b[B\r")
            terminal.command("/dream")
            terminal.command("/unknown")
            terminal.wait_text(b"unknown command")
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 20, 65, 0, 0))
            terminal.rows, terminal.columns = 20, 65
            os.write(master, b"\x1b[200~pasted text\x1b[201~")
            terminal.wait_text(b"pasted text", b"enter send")
            os.write(master, b"\x01\r")
            terminal.wait_text(b"What shall we explore or build?", b"enter send")
        terminal.wait_idle()
        pause_child()
        # Delay a full redraw beyond the old output-draining window at exit.
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 60, 180, 0, 0))
        terminal.rows, terminal.columns = 60, 180
        os.write(master, b"/quit\r")
        terminal.wait_exit()
        resume.join()
        # Ratatui emits cursor-addressed diffs, so typed characters need not be
        # contiguous in the byte stream. The durable transcript proves submission.
        assert b"demo" in output
        assert termios.tcgetattr(slave) == before, (
            "terminal attributes were not restored"
        )
    finally:
        if resume is not None:
            resume.cancel()
            resume.join()
        if child.poll() is None:
            child.kill()
            child.wait()
        os.close(master)
        os.close(slave)


run_smoke(reduced_motion=False, full_session=True)
run_smoke(reduced_motion=True, full_session=False)
