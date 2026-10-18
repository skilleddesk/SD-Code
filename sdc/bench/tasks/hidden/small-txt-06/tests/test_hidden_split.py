import unittest

from textkit import tokenize


class Split(unittest.TestCase):
    def test_new_name(self):
        self.assertFalse(hasattr(tokenize, "words"))
        self.assertEqual(tokenize.split_words("Hi there"), ["hi", "there"])
