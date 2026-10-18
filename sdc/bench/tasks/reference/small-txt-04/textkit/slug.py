import re


def slugify(text, max_length=None):
    """Lower-case, hyphen-separated, ASCII letters and digits only."""
    lowered = text.lower()
    cleaned = re.sub(r"[^a-z0-9]+", "-", lowered)
    slug = cleaned.strip("-")

    if max_length is not None and len(slug) > max_length:
        cut = slug[:max_length + 1]

        if "-" in cut:
            slug = cut.rsplit("-", 1)[0]
        else:
            slug = slug[:max_length]

    return slug.strip("-")
