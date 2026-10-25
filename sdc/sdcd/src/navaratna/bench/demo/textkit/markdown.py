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
            found.append((level, stripped[level:].strip()))

    return found
