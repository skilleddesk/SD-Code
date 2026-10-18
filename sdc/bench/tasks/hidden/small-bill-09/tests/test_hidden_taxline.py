import unittest

from billing.invoice import Invoice
from billing.render import render_invoice


class TaxLine(unittest.TestCase):
    def test_percent_in_the_line(self):
        ten = Invoice("A", tax_percent=10)
        ten.add_line("W", 1000)
        self.assertIn("Tax (10%) 1.00", render_invoice(ten))
        seven = Invoice("A", tax_percent=7.5)
        seven.add_line("W", 1000)
        self.assertIn("Tax (7.5%) 0.75", render_invoice(seven))
        zero = Invoice("A")
        zero.add_line("W", 1000)
        self.assertIn("Tax 0.00", render_invoice(zero))
        self.assertNotIn("(", render_invoice(zero))
