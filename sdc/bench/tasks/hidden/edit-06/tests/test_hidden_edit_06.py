import os
import tempfile
import unittest
from notes.tags import normalize_tag


class Hidden(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.dir.name, 'n.json')

    def tearDown(self):
        self.dir.cleanup()

    def test_dashes(self):
        self.assertEqual(normalize_tag("#Work -- Items"), "work-items")
        self.assertEqual(normalize_tag(" -home- "), "home")
        self.assertEqual(normalize_tag("#Work Items"), "work-items")
