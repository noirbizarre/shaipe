#!/usr/bin/env python3
"""Check that the *workspace* still detects the terminal's graphics protocol.

`scripts/check-terminal-detection.py` guards `shaipe doctor`. This guards the
thing doctor exists to predict, and it guards it against a different bug.

`crossterm`'s `EventStream` spawns a reader on standard input. `Preview::detect`
writes a capability query and reads the terminal's reply from that same
descriptor. If the stream is constructed first, its reader consumes the reply,
`Preview::detect` times out, and the workspace silently falls back to
half-blocks on a terminal that supports Kitty.

Nothing fails. No test goes red. The pictures just get worse — which is exactly
the shape of the stdout-lock bug that shipped twice, arriving from the other
direction now that the event loop is asynchronous.

So: run the real workspace under a pty, once pretending to be a Kitty terminal
that answers and once pretending to be one that does not, and assert it tells
the two apart. A pass means the query happened; asserting only the first would
also pass if everything always said kitty.

Unix only; `pty` does not exist on Windows.
"""

from __future__ import annotations

import fcntl
import os
import select
import struct
import subprocess
import sys
import termios
import time

# Kitty graphics OK, DA1 claiming sixel, cell size, then Device Status Report.
# The DSR comes last: it is what ends ratatui-image's reader loop.
RESPONSE = (
    b"\x1b_Gi=31;OK\x1b\\" b"\x1b[?62;4c" + b"\x1b[6;34;17t" + b"\x1b[0n"
)

# Long enough for a debug build to open a project, detect, and draw a frame.
BUDGET = 8.0


def run_workspace(binary: str, project: str, *, answer: bool) -> str:
    """Open the workspace in a pty, optionally answering its query."""
    pid, fd = pty.fork()
    if pid == 0:
        # A pristine environment: detection must come from the query, not from
        # a variable that happens to be set on the developer's machine.
        for name in (
            "TMUX",
            "TERM_PROGRAM",
            "KITTY_WINDOW_ID",
            "WEZTERM_EXECUTABLE",
            "KONSOLE_VERSION",
        ):
            os.environ.pop(name, None)
        os.environ["TERM"] = "xterm-kitty"
        os.execv(binary, ["shaipe", "tui", project])

    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 1200, 800))

    output = b""
    answered = False
    deadline = time.time() + BUDGET
    while time.time() < deadline:
        readable, _, _ = select.select([fd], [], [], 0.05)
        if readable:
            try:
                chunk = os.read(fd, 1 << 20)
            except OSError:
                break
            if not chunk:
                break
            output += chunk
            if answer and not answered and b"\x1b_G" in output:
                os.write(fd, RESPONSE)
                answered = True

    # `q` quits; the workspace restores the terminal on its way out.
    try:
        os.write(fd, b"q")
        time.sleep(0.4)
    except OSError:
        pass

    try:
        os.kill(pid, 9)
        os.waitpid(pid, 0)
    except OSError:
        pass

    if answer and not answered:
        raise AssertionError(
            "the workspace never sent the capability query.\n\n"
            "`EventStream` is being constructed before `Preview::detect`, so "
            "its reader is eating the terminal's reply. See the ordering in "
            "`run` in src/tui/mod.rs.\n"
            f"--- output ---\n{output.decode('utf-8', 'replace')[:2000]}"
        )

    return output.decode("utf-8", "replace")


def backend_of(screen: str) -> str:
    """Which backend the status line reports."""
    lowered = screen.lower()
    for name in ("kitty", "sixel", "iterm2", "blocks"):
        if name in lowered:
            return name
    return "nothing"


def main() -> int:
    binary = sys.argv[1] if len(sys.argv) > 1 else None
    if binary is None:
        subprocess.run(["cargo", "build", "--quiet"], check=True)
        binary = os.path.join("target", "debug", "shaipe")
    binary = os.path.abspath(binary)

    project = sys.argv[2] if len(sys.argv) > 2 else "logo.svg"

    answering = backend_of(run_workspace(binary, project, answer=True))
    if answering != "kitty":
        print(
            f"a terminal that answers: the workspace chose {answering!r}, not "
            "'kitty'. The capability reply is being consumed by something else "
            "— almost certainly `EventStream`.",
            file=sys.stderr,
        )
        return 1

    silent = backend_of(run_workspace(binary, project, answer=False))
    if silent != "blocks":
        print(
            f"a terminal that stays silent: the workspace chose {silent!r}, not "
            "'blocks'. Detection is reporting a protocol it never confirmed.",
            file=sys.stderr,
        )
        return 1

    print("workspace terminal detection: ok (kitty when answered, blocks when silent)")
    return 0


if __name__ == "__main__":
    if not hasattr(os, "fork"):
        print("skipped: needs a pty", file=sys.stderr)
        sys.exit(0)
    import pty  # noqa: E402  — after the fork check, because Windows lacks it

    sys.exit(main())
