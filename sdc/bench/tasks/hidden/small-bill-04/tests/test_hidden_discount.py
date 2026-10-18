import unittest

from billing.invoice import Invoice


class Discount(unittest.TestCase):
    def test_discount_before_tax(self):
        invoice = Invoice("Acme", tax_percent=10)
        invoice.add_line("Widget", 1000, 2)
        self.assertEqual(invoice.total_cents(), 2200)
        invoice.add_discount(25)
        self.assertEqual(invoice.discount_cents(), 500)
        self.assertEqual(invoice.tax_cents(), 150)
        self.assertEqual(invoice.total_cents(), 1650)
        invoice.add_discount(0)
        self.assertEqual(invoice.total_cents(), 2200)
        with self.assertRaises(ValueError):
            invoice.add_discount(101)
        with self.assertRaises(ValueError):
            invoice.add_discount(-1)
