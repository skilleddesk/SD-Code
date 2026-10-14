import os
import tempfile
import unittest
from notes.store import NoteStore


class Hidden(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.dir.name, 'n.json')

    def tearDown(self):
        self.dir.cleanup()

    def test_update_changes_only_given_fields(self):
        store = NoteStore(self.path)
        note_id = store.add("old", "body", ["a"])
        self.assertTrue(store.update(note_id, title="new"))
        again = NoteStore(self.path).get(note_id)
        self.assertEqual((again["title"], again["body"], again["tags"]), ("new", "body", ["a"]))
        self.assertFalse(store.update("999", body="x"))
