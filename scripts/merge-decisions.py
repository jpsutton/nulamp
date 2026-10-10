#!/usr/bin/env python3
"""Resolves merge conflicts in docs/decisions.md by keeping both sides (rows are append-only)."""
import re
import sys

path = sys.argv[1] if len(sys.argv) > 1 else "docs/decisions.md"
text = open(path, encoding="utf-8").read()
pattern = re.compile(r"<<<<<<< [^\n]*\n(.*?)=======\n(.*?)>>>>>>> [^\n]*\n", re.S)
text = pattern.sub(lambda m: m.group(1) + m.group(2), text)
open(path, "w", encoding="utf-8").write(text)
