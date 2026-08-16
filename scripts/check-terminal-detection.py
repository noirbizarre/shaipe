#!/usr/bin/env python3
"""Check that `shaipe doctor` really queries the terminal.

This exists because the bug it guards against is invisible to `cargo test`.
`ratatui-image` writes its graphics-capability query from a *spawned thread*,
and `Stdout`'s lock is re-entrant only for the thread holding it. A stdout lock
held across the query therefore blocks that thread until its timeout expires:
the query is never sent, no reply arrives, and Shaipe silently falls back to
half-blocks on a terminal that supports Kitty.

That shipped once in `main.rs`, and then again in `doctor.rs` itself — the
diagnostic misdiagnosing its own terminal. Neither could be caught by a unit
test, because both need a real pty and a terminal that answers.

So: run `shaipe doctor` under a pty, pretend to be a Kitty terminal, and assert
it notices. Also assert that a *silent* terminal still degrades to half-blocks,
so a pass here means the query happened rather than that everything says kitty.

Unix only; `pty` does not exist on Windows.
"""

from __future__ import annotations

import fcntl
import os
import pty
import select
import struct
import subprocess
import sys
import termios
import time

# Distinct from ratatui-image's 10x20 fallback, so a report echoing the
# fallback cannot be mistaken for one that read our answer.
CELL_WIDTH, CELL_HEIGHT = 17, 34

# Kitty graphics OK, DA1 claiming sixel, cell size, then Device Status Report.
# The DSR is what ends the reader loop, so it must come last.
RESPONSE = (
    b"\x1b_Gi=31;OK\x1b\\"
    b"\x1b[?62;4c"
    + b"\x1b[6;%d;%dt" % (CELL_HEIGHT, CELL_WIDTH)
    + b"\x1b[0n"
)


def run_doctor(binary: str, *, answer: bool) -> str:
    """Run `shaipe doctor` in a pty, optionally answering its query."""
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
        os.execv(binary, ["shaipe", "doctor"])

    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 110, 1100, 600))

    output = b""
    answered = False
    deadline = time.time() + 15
    while time.time() < deadline:
        readable, _, _ = select.select([fd], [], [], 0.05)
        if not readable:
            continue
        try:
            chunk = os.read(fd, 65536)
        except OSError:
            break
        if not chunk:
            break
        output += chunk
        if answer and not answered and b"_Gi=31" in output:
            os.write(fd, RESPONSE)
            answered = True

    try:
        os.kill(pid, 9)
        os.waitpid(pid, 0)
    except OSError:
        pass

    if answer and not answered:
        raise AssertionError(
            "shaipe never sent the capability query — the stdout lock is being "
            "held across it again. See `write_lines` in src/main.rs.\n"
            f"--- output ---\n{output.decode('utf-8', 'replace')}"
        )

    return output.decode("utf-8", "replace")


def expect(report: str, needle: str, label: str) -> None:
    if needle not in report:
        raise AssertionError(f"{label}: expected {needle!r} in:\n{report}")


def main() -> int:
    binary = sys.argv[1] if len(sys.argv) > 1 else None
    if binary is None:
        subprocess.run(["cargo", "build", "--quiet"], check=True)
        binary = os.path.join("target", "debug", "shaipe")
    binary = os.path.abspath(binary)

    answering = run_doctor(binary, answer=True)
    expect(answering, "backend              kitty", "a terminal that answers")
    expect(
        answering,
        f"cell size            {CELL_WIDTH}x{CELL_HEIGHT} px",
        "a terminal that answers",
    )

    # The negative case matters: without it, a `doctor` that hard-coded
    # "kitty" would pass. It cannot assert *why* it fell back, because
    # `Picker::from_query_stdio` reports no error for a silent terminal.
    silent = run_doctor(binary, answer=False)
    expect(silent, "backend              blocks", "a silent terminal")

    print("terminal detection: ok (kitty when answered, blocks when silent)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
