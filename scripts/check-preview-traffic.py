#!/usr/bin/env python3
"""Check that the workspace does not flood the terminal with images.

`ratatui-image` transmits Kitty images as raw RGBA, so one preview a couple of
hundred pixels on a side is already over a megabyte of escape sequences — and
under tmux every 4096-byte chunk is separately wrapped in a passthrough
sequence. At that size the number of transmissions is the difference between
a workspace that feels instant and one that appears to hang for ten seconds.

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

# A variant's preview fills its pane rather than capping at 512x512 (see
# PLAN.md: "A variant's preview takes the whole pane..."), so how large one
# transmission is depends on the fixed pty size below: 120x40 cells, and a
# preview pane of roughly 58x35 of them. The picker never gets an answer to
# its capability query over this pty, so it falls back to ratatui-image's own
# default font size, 10x20 — making the preview pane's pixel box a fixed
# 580x700, an RGBA transmission of ~1.6 MB raw, ~2.2 MB base64-encoded.
# Comfortably above that, and comfortably below what even the smallest
# possible second transmission — the old fixed 512x512 fallback, rendered
# before the pane's size was known — would add on top of it.
ONE_IMAGE = 3_000_000

# Enough bytes to be an image rather than a frame of borders and text.
AN_IMAGE = 100_000

# How long the workspace is given to produce its first preview.
#
# Generous, and it has to be: this runs beside a compiler and a test suite, and
# a fixed window that has to contain a rasterise is a measurement of how busy
# the machine is. The window that *matters* is the one after the first image —
# see `QUIET`.
PATIENCE = 30.0

# How long to keep watching after an image has gone out.
#
# This is the actual measurement. The regression being guarded is a *second*
# transmission, so what matters is how long the workspace is watched once it
# has drawn something, not how long it took to get there.
QUIET = 2.0

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def run(binary: str, tmux: bool = False) -> int:
    """Run the workspace under a pty and return the bytes it wrote.

    Waits for an image and then for [`QUIET`] seconds more, rather than for a
    fixed span: under load the first render can take longer than any window
    worth calling short, and a run that timed out before the image went out
    reported "no traffic at all" — which reads as a pass for the tmux half of
    the comparison and as a failure for the other.
    """
    pid, fd = pty.fork()
    if pid == 0:
        for name in ("TMUX", "TERM_PROGRAM", "KITTY_WINDOW_ID"):
            os.environ.pop(name, None)
        os.environ["TERM"] = "xterm-kitty"
        if tmux:
            # Enough for the tmux detection every layer uses.
            os.environ["TMUX"] = "/tmp/fake,1,0"
            os.environ["TERM"] = "tmux-256color"
        os.chdir(ROOT)
        # Kitty is forced rather than detected: this measures the expensive
        # path, and the pty will not answer a capability query.
        os.execv(binary, ["shaipe", "tui", "logo.svg", "--preview", "kitty"])

    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 960, 800))

    written = 0
    deadline = time.time() + PATIENCE
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
            # The clock starts when the first image does, and only then.
            if written > AN_IMAGE:
                deadline = min(deadline, time.time() + QUIET)

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
    if opening < AN_IMAGE:
        raise AssertionError(
            f"opening the workspace transmitted {opening / 1e6:.2f} MB, which is "
            f"not an image at all. Either nothing was rendered, or the preview "
            f"backend fell back to half-blocks despite `--preview kitty`."
        )
    if opening > ONE_IMAGE:
        raise AssertionError(
            f"opening the workspace transmitted {opening / 1e6:.2f} MB, which is "
            f"more than one image. The first render is probably happening before "
            f"the first frame has established the preview pane's size."
        )

    # Under tmux every 4 KiB chunk is wrapped in its own passthrough sequence,
    # which is slow enough to be the whole problem — and only there. So the
    # image is transmitted at half resolution when tmux is detected, and the
    # terminal scales it back up.
    through_tmux = run(binary, tmux=True)
    if through_tmux < AN_IMAGE:
        raise AssertionError(
            f"under tmux the workspace transmitted {through_tmux / 1e6:.2f} MB, "
            f"which is not an image at all."
        )
    if through_tmux > opening / 2:
        raise AssertionError(
            f"under tmux the workspace transmitted {through_tmux / 1e6:.2f} MB "
            f"against {opening / 1e6:.2f} MB without it. The transmit scale is "
            f"not being applied; see `Scale::detect` in src/preview/mod.rs."
        )

    print(
        f"preview traffic: ok (direct {opening / 1e6:.2f} MB, "
        f"tmux {through_tmux / 1e6:.2f} MB)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
