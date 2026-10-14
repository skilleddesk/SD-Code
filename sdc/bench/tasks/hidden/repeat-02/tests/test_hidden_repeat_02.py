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

    def test_rename_tag(self):
        store = NoteStore(self.path)
        store.add("a", "", ["work"])
        store.add("b", "", ["work", "home"])
        store.add("c", "", ["home"])
        self.assertEqual(store.rename_tag("#Work", "job"), 2)
        again = NoteStore(self.path)
        self.assertEqual([note["tags"] for note in again.list()], [["job"], ["home", "job"], ["home"]])
