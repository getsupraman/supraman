"""m2x/2: rename Foreman's term platform -> workspace in the given files, case-preserving, bytes in = bytes out
except the replaced words (line endings untouched). Lines holding a gpui/OS meaning are skipped.

usage: python -I rename_workspace.py <file>...
"""
import re
import sys

KEEP = (b"gpui_platform", b"PlatformInput")
WORD = re.compile(rb"platform|Platform|PLATFORM")
MAP = {b"platform": b"workspace", b"Platform": b"Workspace", b"PLATFORM": b"WORKSPACE"}

for path in sys.argv[1:]:
    data = open(path, "rb").read()
    lines = data.splitlines(keepends=True)
    out = []
    n = 0
    for line in lines:
        if any(k in line for k in KEEP):
            out.append(line)
            continue
        new, c = WORD.subn(lambda m: MAP[m.group(0)], line)
        n += c
        out.append(new)
    if n:
        open(path, "wb").write(b"".join(out))
    print(f"{n:4} {path}")
