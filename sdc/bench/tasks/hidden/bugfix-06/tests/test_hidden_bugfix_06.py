import os
import tempfile
import unittest
from notes.search import search
from notes.store import NoteStore


class Hidden(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.dir.name, 'n.json')

    def tearDown(self):
        self.dir.cleanup()

    def test_words_in_any_language(self):
        store = NoteStore(self.path)
        store.add("বাজার তালিকা", "চাল ডাল")
        store.add("Café menu", "")
        store.add("garden", "tomatoes")
        self.assertEqual([n["title"] for n in search(store, "বাজার")], ["বাজার তালিকা"])
        self.assertEqual([n["title"] for n in search(store, "ডাল")], ["বাজার তালিকা"])
        self.assertEqual([n["title"] for n in search(store, "café")], ["Café menu"])
        self.assertEqual([n["title"] for n in search(store, "garden")], ["garden"])
