"""Search: a note scores one point per occurrence of each query word, title words counting double."""

import re
from typing import List

from .store import NoteStore

# \w alone splits Bengali and other Indic words at their vowel signs, so the combining marks are added.
_WORD = re.compile(r"[\w\u0300-\u036f\u0900-\u0dff]+")


def tokenize(text: str) -> List[str]:
    return _WORD.findall(text.lower())


def search(store: NoteStore, query: str) -> List[dict]:
    words = tokenize(query)
    if not words:
        return []
    scored = []
    for note in store.list():
        title, body = tokenize(note["title"]), tokenize(note["body"])
        score = sum(2 * title.count(word) + body.count(word) for word in words)
        if score > 0:
            scored.append((score, note))
    scored.sort(key=lambda pair: (-pair[0], int(pair[1]["id"])))
    return [note for _, note in scored]
