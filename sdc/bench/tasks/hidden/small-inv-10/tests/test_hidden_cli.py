import os
import tempfile
import unittest
from contextlib import redirect_stdout
from io import StringIO

from inventory.cli import main
from inventory.store import InventoryStore


class Cli(unittest.TestCase):
    def test_remove_and_total(self):
        with tempfile.TemporaryDirectory() as folder:
            path = os.path.join(folder, "i.json")
            main(["--file", path, "add", "A", "a", "5", "250"])
            main(["--file", path, "add", "B", "b", "1", "100"])
            buffer = StringIO()
            with redirect_stdout(buffer):
                main(["--file", path, "total"])
            self.assertEqual(buffer.getvalue().strip(), "$13.50")
            main(["--file", path, "remove", "A"])
            self.assertEqual([item.sku for item in InventoryStore(path).all()], ["B"])
