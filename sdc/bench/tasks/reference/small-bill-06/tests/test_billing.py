import unittest

from billing.discounts import apply_percent_discount, bulk_discount_percent
from billing.invoice import Invoice
from billing.money import percentage_of
from billing.render import render_invoice


class BillingTest(unittest.TestCase):
    def test_percent_rounding(self):
        self.assertEqual(percentage_of(1000, 7.5), 75)
        self.assertEqual(percentage_of(199, 10), 20)

    def test_invoice_totals(self):
        invoice = Invoice("Acme", tax_percent=10)
        invoice.add_line("Widget", 250, 4)
        self.assertEqual(invoice.subtotal_cents(), 1000)
        self.assertEqual(invoice.total_cents(), 1100)

    def test_discounts(self):
        self.assertEqual(apply_percent_discount(2000, 10), 1800)
        self.assertEqual(bulk_discount_percent(3), 0)

    def test_render(self):
        invoice = Invoice("Acme")
        invoice.add_line("Widget", 250, 2)
        self.assertIn("Total 5.00", render_invoice(invoice))


if __name__ == "__main__":
    unittest.main()
