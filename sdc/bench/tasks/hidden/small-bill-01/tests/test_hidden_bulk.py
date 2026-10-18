import unittest

from billing.discounts import bulk_discount_percent


class Bulk(unittest.TestCase):
    def test_edges(self):
        self.assertEqual([bulk_discount_percent(n) for n in (9, 10, 49, 50, 51)], [0, 5, 5, 10, 10])
