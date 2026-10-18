import unittest

from inventory.reports import format_money


class Negative(unittest.TestCase):
    def test_negative(self):
        self.assertEqual(format_money(-5), "-$0.05")
        self.assertEqual(format_money(-12345), "-$123.45")
        self.assertEqual(format_money(0), "$0.00")
        self.assertEqual(format_money(12345), "$123.45")
