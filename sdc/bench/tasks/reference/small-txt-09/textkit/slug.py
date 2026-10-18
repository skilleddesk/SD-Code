import re
import unicodedata


def slugify(text):
    """Lower-case, hyphen-separated, ASCII letters and digits only."""
    plain = unicodedata.normalize("NFKD", text).encode("ascii", "ignore").decode("ascii")
    lowered = plain.lower()
    cleaned = re.sub(r"[^a-z0-9]+", "-", lowered)
    return cleaned.strip("-")
