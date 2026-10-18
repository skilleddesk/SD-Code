def wrap_text(text, width):
    """Greedy word wrap to at most `width` characters per line."""
    lines = []
    current = ""

    for word in text.split():
        if not current:
            current = word
        elif len(current) + 1 + len(word) < width:
            current += " " + word
        else:
            lines.append(current)
            current = word

    if current:
        lines.append(current)

    return lines
