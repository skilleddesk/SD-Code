import unittest

from inventory.reports import format_money


class Money(unittest.TestCase):
    def test_thousands(self):
        self.assertEqual(format_money(1234567), "$12,345.67")
        self.assertEqual(format_money(99), "$0.99")
        self.assertEqual(format_money(100000), "$1,000.00")
        self.assertEqual(format_money(12345), "$123.45")
