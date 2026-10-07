import glob
import os
import re
import sys

E = os.path.dirname(os.path.abspath(__file__))
FACT = re.compile(r"\b[0-9a-f]{7}\b|#\d+|\b\d+(?:\.\d+)?[A-Za-z%]*\b|[\w.-]+/[\w./*-]+|\"[^\"]{3,60}\"")


def level(m, l):
    out = {}
    for p in sorted(glob.glob(f"{E}/t-{m}/tree/{l}/*/*")):
        seg = int(p.split("/")[-2], 16) * 256 + int(p.split("/")[-1], 16)
        for k, line in enumerate(open(p).read().split("\n")[:-1]):
            if line:
                out[seg * 256 + k] = line.split(" ", 5)[5]
    return out


def facts(s):
    return set(FACT.findall(s))


for m in sys.argv[1:]:
    leaves = level(m, 0)
    rows = []
    for l in range(1, 7):
        for i, text in level(m, l).items():
            lo, hi = i << l, (i + 1) << l
            src = set().union(*(facts(leaves[j]) for j in range(lo, hi) if j in leaves))
            if src:
                rows.append((l, len(src & facts(text)) / len(src)))
    for l in range(1, 7):
        r = [k for lv, k in rows if lv == l]
        if r:
            print(f"{m} level {l} ({1 << l} notes): {100 * sum(r) / len(r):.0f}% of hard facts kept, {len(r)} lines")
