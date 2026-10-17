"""python -m notes.cli --db notes.json add|list|search ..."""

import argparse
import sys
from typing import List, Optional

from .dates import parse_due
from .search import search
from .store import NoteStore


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(prog="notes")
    parser.add_argument("--db", default="notes.json")
    commands = parser.add_subparsers(dest="command", required=True)

    add = commands.add_parser("add")
    add.add_argument("title")
    add.add_argument("--body", default="")
    add.add_argument("--tag", action="append", default=[])
    add.add_argument("--due")
    add.add_argument("--priority", type=int, default=3)

    listing = commands.add_parser("list")
    listing.add_argument("--tag")

    find = commands.add_parser("search")
    find.add_argument("query")

    args = parser.parse_args(argv)
    store = NoteStore(args.db)

    if args.command == "add":
        due = parse_due(args.due).isoformat() if args.due else None
        print(store.add(args.title, args.body, args.tag, due, args.priority))
    elif args.command == "list":
        for note in store.list(args.tag):
            print(f"{note['id']}\t{note['title']}\tP{note['priority']}")
    elif args.command == "search":
        for note in search(store, args.query):
            print(f"{note['id']}\t{note['title']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
