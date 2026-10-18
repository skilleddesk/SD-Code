import os
import tempfile
import unittest

from inventory import reports
from inventory.store import InventoryStore


class Renamed(unittest.TestCase):
    def test_new_name_only(self):
        self.assertFalse(hasattr(reports, "by_price_band"))
        with tempfile.TemporaryDirectory() as folder:
            store = InventoryStore(os.path.join(folder, "i.json"))
            store.add("a", "a", 1, 5)
            self.assertEqual(reports.price_bands(store)["cheap"], ["a"])
