import os
import tempfile
import unittest
from notes.export import export_markdown
from notes.store import NoteStore


class Hidden(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.dir.name, 'n.json')

    def tearDown(self):
        self.dir.cleanup()

    def test_markdown(self):
        store = NoteStore(self.path)
        store.add("Groceries", "milk, eggs", ["shopping", "home"], "2026-10-20")
        store.add("Idea", "")
        text = export_markdown(store)
        self.assertTrue(text.endswith("\n") and not text.endswith("\n\n"))
        self.assertLess(text.index("## Groceries"), text.index("## Idea"))
        self.assertIn("Tags: home, shopping", text)
        self.assertIn("Due: 2026-10-20", text)
        self.assertIn("milk, eggs", text)
        self.assertEqual(text.count("Tags:"), 1)
        self.assertEqual(text.count("Due:"), 1)
