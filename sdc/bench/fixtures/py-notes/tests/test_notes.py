import datetime
import os
import tempfile
import unittest

from notes.dates import parse_due
from notes.search import search
from notes.store import NoteStore
from notes.tags import normalize_tag


class NotesTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.dir.name, "notes.json")

    def tearDown(self):
        self.dir.cleanup()

    def test_store_round_trip(self):
        store = NoteStore(self.path)
        note_id = store.add("Buy milk", "two litres", ["#Home"])
        again = NoteStore(self.path)
        self.assertEqual(again.get(note_id)["tags"], ["home"])
        self.assertTrue(again.delete(note_id))
        self.assertEqual(NoteStore(self.path).list(), [])

    def test_tags_and_dates(self):
        self.assertEqual(normalize_tag(" #Work Items "), "work-items")
        today = datetime.date(2026, 10, 14)
        self.assertEqual(parse_due("in 3 days", today), datetime.date(2026, 10, 17))
        self.assertEqual(parse_due("2026-12-01", today), datetime.date(2026, 12, 1))

    def test_search_ranks_title_words_higher(self):
        store = NoteStore(self.path)
        store.add("garden plan", "water the tomatoes")
        store.add("shopping", "garden hose, garden gloves, garden soil")
        store.add("unrelated", "nothing here")
        self.assertEqual([note["title"] for note in search(store, "garden")], ["shopping", "garden plan"])


if __name__ == "__main__":
    unittest.main()
