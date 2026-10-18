import unittest

from textkit.stats import top_n


class Stop(unittest.TestCase):
    def test_stopwords(self):
        text = "the cat and the dog and the bird"
        self.assertEqual(top_n(text, 2), [("the", 3), ("and", 2)])
        self.assertEqual(top_n(text, 2, stopwords=("the", "AND")), [("bird", 1), ("cat", 1)])
        self.assertEqual(top_n(text, 5, ["the"])[0], ("and", 2))
