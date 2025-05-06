#!/usr/bin/env python3
import re
import sys
from collections import defaultdict


def parse_suggestions(sugg_text):
    """
    Parse lines like:
      path/to/file.rs:484  // append `.map_mm_err(GenTxError::from)?`
    Returns a dict[file] -> list of (line_idx0, target)
    """
    out = defaultdict(list)
    for ln in sugg_text.splitlines():
        m = re.match(r"([^:]+):(\d+)\s+//\s+append `(.+?)`", ln)
        if not m:
            continue
        path, lnum, call = m.groups()
        # extract the part inside .map_mm_err(...):
        # we'll drop the trailing '?' in the suggestion because we insert it back
        call = call.rstrip("?")
        out[path].append((int(lnum) - 1, call))
    return out


def apply_patches(patches):
    """
    patches: dict[file] -> list of (lineno0, insert_call)
    """
    for path, edits in patches.items():
        print(f"Patching {path}…")
        # read all lines
        with open(path, "r") as f:
            lines = f.readlines()

        # apply each edit
        for ln, call in edits:
            if ln < 0 or ln >= len(lines):
                print(f"  ⚠️  invalid line {ln + 1}")
                continue
            orig = lines[ln]
            # replace first occurrence of `)?` with `).map_mm_err()?`
            new = re.sub(r"\?(?=\s|;|,|$)", ".map_mm_err()?", orig, count=1)
            if new == orig:
                print(f"  ⚠️  no ')?' found on line {ln + 1}")
            else:
                lines[ln] = new
                print(f"  ✓  line {ln + 1}: inserted `.map_mm_err()`")

        # write back
        with open(path, "w") as f:
            f.writelines(lines)


def main():
    if len(sys.argv) != 2:
        print("Usage: apply_map_mmerrs.py <suggestions.txt>", file=sys.stderr)
        sys.exit(1)

    sugg_text = open(sys.argv[1]).read()
    patches = parse_suggestions(sugg_text)
    apply_patches(patches)


if __name__ == "__main__":
    main()
