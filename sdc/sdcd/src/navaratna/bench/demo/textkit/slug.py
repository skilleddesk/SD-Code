import re


def slugify(text):
    """Lower-case, hyphen-separated, ASCII letters and digits only."""
    lowered = text.lower()
    cleaned = re.sub(r"[^a-z0-9]+", "-", lowered)
    return cleaned.strip("-")
