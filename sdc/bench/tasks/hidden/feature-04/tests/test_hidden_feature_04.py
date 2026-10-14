import os
import tempfile
import unittest
import subprocess
import sys

from notes.store import NoteStore


class Hidden(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.dir.name, 'n.json')

    def tearDown(self):
        self.dir.cleanup()

    def test_tags_command(self):
        store = NoteStore(self.path)
        store.add("a", "", ["work", "home"])
        store.add("b", "", ["work"])
        store.add("c", "", ["errand"])
        out = subprocess.run([sys.executable, "-m", "notes.cli", "--db", self.path, "tags"], capture_output=True, text=True, check=True).stdout
        self.assertEqual(out.split("\n")[:3], ["work 2", "errand 1", "home 1"])
