import unittest

from billing.invoice import Invoice


class AsDict(unittest.TestCase):
    def test_to_dict(self):
        invoice = Invoice("Acme", tax_percent=10)
        invoice.add_line("Widget", 250, 4)
        self.assertEqual(
            invoice.to_dict(),
            {
                "customer": "Acme",
                "lines": [{"description": "Widget", "unit_cents": 250, "quantity": 4}],
                "subtotal_cents": 1000,
                "tax_cents": 100,
                "total_cents": 1100,
            },
        )
