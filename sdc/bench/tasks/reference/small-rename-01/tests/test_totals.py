import unittest

from pkg.core import compute_grand_total
from pkg.mod_07 import total_07


class Totals(unittest.TestCase):
    def test_core(self):
        self.assertEqual(compute_grand_total([1, 2, 3]), 6)

    def test_report(self):
        self.assertEqual(total_07([1, 2]), 10)
