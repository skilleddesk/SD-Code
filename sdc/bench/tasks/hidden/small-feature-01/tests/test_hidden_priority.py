import io
import os
import tempfile
import unittest
from contextlib import redirect_stdout

from notes.cli import main
from notes.search import search
from notes.store import NoteStore


class Priority(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.dir.name, "n.json")

    def tearDown(self):
        self.dir.cleanup()

    def test_store_keeps_priority_with_a_default_of_3(self):
        store = NoteStore(self.path)
        a = store.add("plain")
        b = store.add("urgent", priority=1)
        again = NoteStore(self.path)
        self.assertEqual(again.get(a)["priority"], 3)
        self.assertEqual(again.get(b)["priority"], 1)

    def test_an_old_file_without_priority_still_loads_as_3(self):
        with open(self.path, "w", encoding="utf-8") as handle:
            handle.write('{"notes": {"1": {"title": "old", "body": "", "tags": [], "due": null}}, "next_id": 2}')
        self.assertEqual(NoteStore(self.path).list()[0]["priority"], 3)

    def test_priority_must_be_1_to_5(self):
        store = NoteStore(self.path)
        for bad in (0, 6, -1):
            with self.assertRaises(ValueError):
                store.add("x", priority=bad)

    def test_equal_search_scores_are_ordered_by_priority_then_id(self):
        store = NoteStore(self.path)
        store.add("report draft", priority=4)
        store.add("report final", priority=1)
        store.add("report notes", priority=1)
        self.assertEqual([n["title"] for n in search(store, "report")], ["report final", "report notes", "report draft"])

    def test_the_cli_takes_a_priority_flag_and_list_shows_it(self):
        main(["--db", self.path, "add", "ship it", "--priority", "2"])
        out = io.StringIO()
        with redirect_stdout(out):
            main(["--db", self.path, "list"])
        self.assertEqual(out.getvalue().strip(), "1\tship it\tP2")
