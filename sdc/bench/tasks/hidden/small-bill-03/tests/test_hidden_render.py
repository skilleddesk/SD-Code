import unittest

from billing.render import money


class Money(unittest.TestCase):
    def test_negative(self):
        self.assertEqual(money(-5), "-0.05")
        self.assertEqual(money(-12345), "-123.45")
        self.assertEqual(money(5), "0.05")
        self.assertEqual(money(0), "0.00")
