#!/usr/bin/env python3
"""Wait until the Zed window is quiet, then capture a PNG.

Polls grim frames at 2 s intervals. A frame counts as a real change only
when more than MIN_DIFF bytes differ from the previous frame; the blinking
caret and tiny status-bar changes stay under that. The wait ends when the
screen has been quiet for QUIET seconds, or at the timeout.

Usage: poll.py OUT.png [TIMEOUT_S [QUIET_S]]
Exit 0 when the quiet period was reached, 1 on timeout.
"""
import subprocess
import sys
import time
from PIL import Image

W, H = 1600, 1000
MIN_DIFF = 1500  # bytes; ~500 pixels
PNG = "/tmp/poll-frame.png"


def frame() -> bytes:
    subprocess.run(["grim", PNG], check=True)
    img = Image.open(PNG).convert("RGB")
    if img.size != (W, H):
        img = img.resize((W, H))
    return img.tobytes()


def diff_bytes(a: bytes, b: bytes) -> int:
    n = 0
    for i in range(0, len(a), 1024):
        sa, sb = a[i:i + 1024], b[i:i + 1024]
        if sa != sb:
            n += sum(1 for x, y in zip(sa, sb) if x != y)
    return n


def main() -> None:
    out = sys.argv[1]
    timeout = float(sys.argv[2]) if len(sys.argv) > 2 else 300.0
    quiet = float(sys.argv[3]) if len(sys.argv) > 3 else 8.0
    deadline = time.monotonic() + timeout
    prev = None
    last_change = time.monotonic()
    while time.monotonic() < deadline:
        cur = frame()
        if prev is not None:
            if diff_bytes(cur, prev) > MIN_DIFF:
                last_change = time.monotonic()
        prev = cur
        if time.monotonic() - last_change >= quiet:
            subprocess.run(["grim", out], check=True)
            print(f"QUIET after {time.monotonic() - last_change:.0f}s: {out}")
            return
        time.sleep(2)
    subprocess.run(["grim", out], check=True)
    print(f"TIMEOUT after {timeout:.0f}s: {out}")
    sys.exit(1)


if __name__ == "__main__":
    main()
