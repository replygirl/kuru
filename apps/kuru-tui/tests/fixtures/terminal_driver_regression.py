"""Exercise real PTY backpressure and bounded failure diagnostics."""

import os
import pty
import subprocess
import sys
import unittest

from terminal_driver import Terminal


class TerminalDriverTests(unittest.TestCase):
    def run_child(self, script):
        master, slave = pty.openpty()
        child = subprocess.Popen(
            [sys.executable, "-c", script], stdin=slave, stdout=slave, stderr=slave
        )

        def cleanup():
            if child.poll() is None:
                child.kill()
                child.wait()
            os.close(master)
            os.close(slave)

        self.addCleanup(cleanup)
        return Terminal(master, child)

    def test_shutdown_drains_delayed_output_larger_than_terminal_buffer(self):
        terminal = self.run_child(
            "import sys,time; time.sleep(0.7); "
            "sys.stdout.write('x' * 1048576 + 'DONE'); sys.stdout.flush()"
        )
        terminal.wait_exit()
        self.assertEqual(bytes(terminal.output), b"x" * 1048576 + b"DONE")

    def test_stalled_process_fails_with_bounded_screen_diagnostic(self):
        terminal = self.run_child(
            "import sys,time; print('STALLED', flush=True); time.sleep(30)"
        )
        terminal.wait_text(b"STALLED")
        with self.assertRaisesRegex(AssertionError, "process exits.*STALLED"):
            terminal.wait_exit(timeout=0.1)


unittest.main()
