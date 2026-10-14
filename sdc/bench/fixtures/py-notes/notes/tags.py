"""Tags are compared in one canonical form."""


def normalize_tag(tag: str) -> str:
    """'#Work Items' -> 'work-items'."""
    return tag.strip().lstrip("#").lower().replace(" ", "-")
