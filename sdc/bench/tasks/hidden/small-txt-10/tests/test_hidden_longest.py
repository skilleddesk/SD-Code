import unittest

from textkit.stats import longest_word


class Longest(unittest.TestCase):
    def test_longest(self):
        self.assertEqual(longest_word("a bbb cc ddd"), "bbb")
        self.assertEqual(longest_word("..."), "")
        self.assertEqual(longest_word("Short LONGER"), "longer")
