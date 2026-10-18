import re

_WORD = re.compile(r"[A-Za-z0-9']+")


def split_words(text):
    """Lower-case words of a text, in order."""
    return [match.group(0).lower() for match in _WORD.finditer(text)]


def sentences(text):
    """Split on ., ! or ? followed by whitespace; keeps the punctuation with its sentence."""
    parts = re.split(r"(?<=[.!?])\s+", text.strip())
    return [part for part in parts if part]
