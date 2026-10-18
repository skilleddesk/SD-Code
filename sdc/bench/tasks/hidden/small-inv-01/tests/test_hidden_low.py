import os
import tempfile
import unittest
from contextlib import redirect_stdout
from io import StringIO

from inventory.cli import main
from inventory.store import InventoryStore


class Low(unittest.TestCase):
    def test_store_and_cli(self):
        with tempfile.TemporaryDirectory() as folder:
            path = os.path.join(folder, "i.json")
            store = InventoryStore(path)
            store.add("B", "b", 2, 100)
            store.add("A", "a", 2, 100)
            store.add("C", "c", 9, 100)
            store.add("D", "d", 0, 100)
            self.assertEqual([item.sku for item in store.low_stock(5)], ["D", "A", "B"])
            self.assertEqual(store.low_stock(0), [])
            buffer = StringIO()
            with redirect_stdout(buffer):
                main(["--file", path, "low", "5"])
            self.assertEqual(buffer.getvalue().splitlines(), ["D\t0", "A\t2", "B\t2"])
