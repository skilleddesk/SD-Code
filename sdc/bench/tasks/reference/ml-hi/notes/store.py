"""Notes kept in one JSON file. Every change is written to disk at once."""

import json
import os
from typing import Dict, List, Optional

from .tags import normalize_tag


class NoteStore:
    def __init__(self, path: str):
        self.path = path
        self.notes: Dict[str, dict] = {}
        self.next_id = 1
        if os.path.exists(path):
            self._load()

    def _load(self) -> None:
        with open(self.path, encoding="utf-8") as handle:
            data = json.load(handle)
        self.notes = data.get("notes", {})
        self.next_id = data.get("next_id", len(self.notes) + 1)

    def save(self) -> None:
        """Writes the whole store, through a temporary file so a crash never leaves half a file."""
        temporary = self.path + ".tmp"
        with open(temporary, "w", encoding="utf-8") as handle:
            json.dump({"notes": self.notes, "next_id": self.next_id}, handle, ensure_ascii=False, indent=2)
        os.replace(temporary, self.path)

    def add(self, title: str, body: str = "", tags: Optional[List[str]] = None, due: Optional[str] = None) -> str:
        note_id = str(self.next_id)
        self.next_id += 1
        self.notes[note_id] = {
            "title": title,
            "body": body,
            "tags": sorted({normalize_tag(tag) for tag in (tags or [])}),
            "due": due,
        }
        self.save()
        return note_id

    def get(self, note_id: str) -> Optional[dict]:
        return self.notes.get(note_id)

    def delete(self, note_id: str) -> bool:
        if note_id not in self.notes:
            return False
        del self.notes[note_id]
        self.save()
        return True

    def count_by_tag(self) -> Dict[str, int]:
        counts: Dict[str, int] = {}
        for note in self.notes.values():
            for tag in note["tags"]:
                counts[tag] = counts.get(tag, 0) + 1
        return counts

    def list(self, tag: Optional[str] = None) -> List[dict]:
        wanted = normalize_tag(tag) if tag else None
        return [
            dict(note, id=note_id)
            for note_id, note in sorted(self.notes.items(), key=lambda item: int(item[0]))
            if wanted is None or wanted in note["tags"]
        ]
