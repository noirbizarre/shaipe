#!/usr/bin/env python3
"""Check that the workspace does not flood the terminal with images.

`ratatui-image` transmits Kitty images as raw RGBA, so one preview of a
512x512 image is about 1.4 MB of escape sequences — and under tmux every
4096-byte chunk is separately wrapped in a passthrough sequence. At that size
the number of transmissions is the difference between a workspace that feels
instant and one that appears to hang for ten seconds.

Two regressions are guarded here, both of which shipped:

- opening the workspace rendered *twice*, because the first render ran before
  the first frame had established how large the preview pane was;
- a burst of keypresses rendered once per key, because the event loop read a
  single event per iteration and never drained the queue.

Only the first is checked here. Counting bytes is the only way to see it: it is
a property of what a real terminal is asked to swallow, and invisible to
`cargo test`. The second is a property of the decision logic, and is checked
deterministically by `a_burst_of_keys_costs_one_render_not_one_per_key` in
`src/tui/app.rs` — driving keys through a pty to assert it turned out to test
the harness more than the program.

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

# One 512x512 RGBA transmission is ~1.4 MB. Anything above this means more than
# one image went out; comfortably above a single transmission's worth of
# placeholder redraws.
ONE_IMAGE = 1_500_000

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def run(binary: str, keys: bytes = b"", settle: float = 6.0, at: float = 4.0) -> int:
    """Run the workspace under a pty and return the bytes it wrote."""
    pid, fd = pty.fork()
    if pid == 0:
        for name in ("TMUX", "TERM_PROGRAM", "KITTY_WINDOW_ID"):
            os.environ.pop(name, None)
        os.environ["TERM"] = "xterm-kitty"
        os.chdir(ROOT)
        # Kitty is forced rather than detected: this measures the expensive
        # path, and the pty will not answer a capability query.
        os.execv(binary, ["shaipe", "tui", "logo.svg", "--preview", "kitty"])

    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 960, 800))

    written = 0
    deadline = time.time() + settle
    sent = False
    while time.time() < deadline:
        readable, _, _ = select.select([fd], [], [], 0.05)
        if readable:
            try:
                chunk = os.read(fd, 1 << 20)
            except OSError:
                break
            if not chunk:
                break
            written += len(chunk)
        if keys and not sent and time.time() - (deadline - settle) > at:
            os.write(fd, keys)
            sent = True

    try:
        os.kill(pid, 9)
        os.waitpid(pid, 0)
    except OSError:
        pass
    return written


def main() -> int:
    binary = sys.argv[1] if len(sys.argv) > 1 else None
    if binary is None:
        subprocess.run(["cargo", "build", "--quiet"], check=True, cwd=ROOT)
        binary = os.path.join(ROOT, "target", "debug", "shaipe")
    binary = os.path.abspath(binary)

    opening = run(binary)
    if opening > ONE_IMAGE:
        raise AssertionError(
            f"opening the workspace transmitted {opening / 1e6:.2f} MB, which is "
            f"more than one image. The first render is probably happening before "
            f"the first frame has established the preview pane's size."
        )

    print(f"preview traffic: ok (opening transmits {opening / 1e6:.2f} MB, one image)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
