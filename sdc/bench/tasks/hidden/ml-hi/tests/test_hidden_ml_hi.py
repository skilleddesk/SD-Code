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

    def test_count_by_tag(self):
        store = NoteStore(self.path)
        store.add("a", "", ["work", "home"])
        store.add("b", "", ["work"])
        store.add("c", "")
        self.assertEqual(store.count_by_tag(), {"work": 2, "home": 1})
