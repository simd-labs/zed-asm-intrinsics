#!/usr/bin/env python3
"""Find the y centers of editor lines via gutter line-number text.

Usage: measure.py [IN.png]  (default: /shot/probe.png)

Aims the hover at a measured line center, not a guessed one.
"""
import sys
from PIL import Image

path = sys.argv[1] if len(sys.argv) > 1 else "/shot/probe.png"
im = Image.open(path).convert("L")
px = im.load()
W, H = im.size
print("size", im.size)

rows = []
for y in range(90, 400):
    n = sum(1 for x in range(10, 140) if px[x, y] > 55)
    rows.append((y, n))
clusters = []
cur = []
for y, n in rows:
    if n >= 2:
        cur.append(y)
    elif cur:
        clusters.append((cur[0], cur[-1], (cur[0] + cur[-1]) // 2))
        cur = []
if cur:
    clusters.append((cur[0], cur[-1], (cur[0] + cur[-1]) // 2))
print("gutter text clusters (start, end, center):")
for c in clusters:
    print(c)
