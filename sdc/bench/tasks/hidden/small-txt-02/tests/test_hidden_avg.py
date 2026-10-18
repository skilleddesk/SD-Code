import unittest

from textkit.stats import average_word_length


class Average(unittest.TestCase):
    def test_empty(self):
        self.assertEqual(average_word_length(""), 0.0)
        self.assertEqual(average_word_length("... !!!"), 0.0)
        self.assertEqual(average_word_length("ab abcd"), 3.0)
