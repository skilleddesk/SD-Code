import unittest

from scripts.report import summary
from scripts.stats import mean, median, spread
from scripts.text import shout, words


class Median(unittest.TestCase):
    def test_even_length_is_the_mean_of_the_middle_two(self):
        self.assertEqual(median([4, 1, 3, 2]), 2.5)
        self.assertEqual(median([10, 20]), 15)

    def test_odd_length_and_one_value_are_unchanged(self):
        self.assertEqual(median([5, 1, 3]), 3)
        self.assertEqual(median([7]), 7)

    def test_nothing_else_changed(self):
        self.assertEqual(mean([1, 2, 3]), 2)
        self.assertEqual(spread([1, 9, 4]), 8)
        self.assertEqual(shout("hi"), "HI!")
        self.assertEqual(words("a b  c"), ["a", "b", "c"])
        self.assertEqual(summary([1, 2, 3, 4]), "MEAN 2.5 MEDIAN 2.5!")

    def test_the_input_is_not_reordered(self):
        values = [3, 1, 2]
        median(values)
        self.assertEqual(values, [3, 1, 2])
