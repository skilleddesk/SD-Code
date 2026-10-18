def headings(text):
    """(level, title) for each ATX heading line (# .. ######) outside fenced code blocks."""
    found = []
    fenced = False

    for line in text.splitlines():
        if line.startswith("```"):
            fenced = not fenced
            continue

        if fenced:
            continue

        stripped = line.strip()
        level = len(stripped) - len(stripped.lstrip("#"))

        if 1 <= level <= 6 and stripped[level:level + 1] == " ":
            title = stripped[level:].strip()
            closed = title.rstrip("#")

            if closed != title and (closed == "" or closed.endswith(" ")):
                title = closed.rstrip()

            found.append((level, title))

    return found
