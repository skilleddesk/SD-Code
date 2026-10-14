"""Markdown export."""

from .store import NoteStore


def export_markdown(store: NoteStore) -> str:
    blocks = []
    for note in store.list():
        lines = [f"## {note['title']}"]
        if note["tags"]:
            lines.append("Tags: " + ", ".join(note["tags"]))
        if note.get("due"):
            lines.append(f"Due: {note['due']}")
        if note["body"]:
            lines.append(note["body"])
        blocks.append("\n".join(lines))
    return "\n\n".join(blocks) + "\n"
