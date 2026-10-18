import unittest

from textkit.wrap import wrap_text


class Wrap(unittest.TestCase):
    def test_exact_width(self):
        self.assertEqual(wrap_text("aaa bbb", 7), ["aaa bbb"])
        self.assertEqual(wrap_text("aaa bbb ccc", 7), ["aaa bbb", "ccc"])
        self.assertEqual(wrap_text("abcdefgh", 4), ["abcdefgh"])
