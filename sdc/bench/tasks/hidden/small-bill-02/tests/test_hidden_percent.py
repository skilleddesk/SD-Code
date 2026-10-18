import unittest

from billing.money import percent_of


class Percent(unittest.TestCase):
    def test_one_decimal(self):
        self.assertEqual(percent_of(1000, 8.2), 82)
        self.assertEqual(percent_of(1000, 4.6), 46)
        self.assertEqual(percent_of(1000, 7.5), 75)
        self.assertEqual(percent_of(200, 0.7), 1)
        self.assertEqual(percent_of(1000, 10), 100)
