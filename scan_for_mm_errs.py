#!/usr/bin/env python3
import re
import sys


def extract_map_mmerrs(log_text):
    """
    Yields tuples (file, line, inner, target) for each E0277 `?` conversion error.
    """
    lines = log_text.splitlines()
    i = 0
    while i < len(lines):
        if (
            "error[E0277]" in lines[i]
            and "`?` couldn't convert the error to" in lines[i]
        ):
            # 1) capture the target from this line
            m_to = re.search(r"couldn't convert the error to `([^`]+)`", lines[i])
            target = m_to.group(1) if m_to else None

            # 2) find the file:line arrow below
            file_path, line_no = None, None
            j = i + 1
            while j < len(lines):
                m_loc = re.match(r"\s*-->\s+([^:]+):(\d+):\d+", lines[j])
                if m_loc:
                    file_path, line_no = m_loc.group(1), m_loc.group(2)
                    break
                j += 1

            # 3) find the From<…> trait bound line for inner type
            inner = None
            k = j + 1
            while k < len(lines) and inner is None:
                m_from = re.search(r"From<[^<]*<([^>]+)>", lines[k])
                if m_from:
                    inner = m_from.group(1)
                k += 1

            if file_path and line_no and inner and target:
                yield file_path, line_no, inner, target

            i = k
        else:
            i += 1


def main():
    if len(sys.argv) != 2:
        print("Usage: extract_map_mmerrs.py <rust_errors.log>", file=sys.stderr)
        sys.exit(1)

    log = open(sys.argv[1]).read()
    for path, line, inner, target in extract_map_mmerrs(log):
        print(
            f"{path}:{line} // append `.map_mm_err({target}.from)?` (inner was `{inner}`)"
        )


if __name__ == "__main__":
    main()
