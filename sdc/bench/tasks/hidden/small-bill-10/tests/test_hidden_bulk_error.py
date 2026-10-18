import unittest

from billing.discounts import bulk_discount_percent


class BulkError(unittest.TestCase):
    def test_rejects_nonsense(self):
        for bad in (0, -1):
            with self.assertRaises(ValueError):
                bulk_discount_percent(bad)
        self.assertEqual(bulk_discount_percent(1), 0)
        self.assertEqual(bulk_discount_percent(50), 10)
