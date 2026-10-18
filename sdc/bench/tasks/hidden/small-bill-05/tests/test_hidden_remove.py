import unittest

from billing.invoice import Invoice


class Remove(unittest.TestCase):
    def test_remove(self):
        invoice = Invoice("Acme")
        invoice.add_line("Widget", 100, 2)
        invoice.add_line("Gadget", 50)
        invoice.add_line("Widget", 10)
        self.assertEqual(invoice.remove_line("Widget"), 2)
        self.assertEqual(invoice.subtotal_cents(), 50)
        self.assertEqual(invoice.remove_line("Nothing"), 0)
