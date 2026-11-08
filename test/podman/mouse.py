#!/usr/bin/env python3
"""Move the pointer with the wlr virtual pointer protocol (wayland-automation).

Usage: mouse.py x y  (coordinates in screen pixels, 1:1)

The library scales motion-absolute by a detected resolution; detection fails
in this container and falls back to 1920x1080, so we pin it to the real
1600x1000 output.
"""
import sys

import wayland_automation.mouse_controller as mc

mc.get_resolution = lambda: (1600, 1000)
Mouse = mc.Mouse


def main() -> None:
    x, y = int(sys.argv[1]), int(sys.argv[2])
    with Mouse() as mouse:
        mouse.click(x, y, "nothing")  # "nothing" = move only, no button
    print(f"moved to {x} {y}")


if __name__ == "__main__":
    main()
