"""Tags are compared in one canonical form."""

import re


def normalize_tag(tag: str) -> str:
    """'#Work Items' -> 'work-items'."""
    text = tag.strip().lstrip("#").lower().replace(" ", "-")
    return re.sub(r"-+", "-", text).strip("-")
