"""Search: a note scores one point per occurrence of each query word, title words counting double."""

import re
from typing import List

from .store import NoteStore

_WORD = re.compile(r"[a-z0-9]+")


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
    scored.sort(key=lambda pair: (-pair[0], pair[1]["priority"], int(pair[1]["id"])))
    return [note for _, note in scored]
