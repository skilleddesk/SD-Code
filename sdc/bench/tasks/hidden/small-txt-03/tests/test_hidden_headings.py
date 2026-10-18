import unittest

from textkit.markdown import headings


class Headings(unittest.TestCase):
    def test_closing_hashes(self):
        self.assertEqual(headings("## Setup ##"), [(2, "Setup")])
        self.assertEqual(headings("# C#"), [(1, "C#")])
        self.assertEqual(headings("### Deep ###   "), [(3, "Deep")])
        self.assertEqual(headings("# Plain"), [(1, "Plain")])
