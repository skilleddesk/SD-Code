import unittest

from billing import money


class Renamed(unittest.TestCase):
    def test_new_name(self):
        self.assertFalse(hasattr(money, "percent_of"))
        self.assertEqual(money.percentage_of(1000, 7.5), 75)
