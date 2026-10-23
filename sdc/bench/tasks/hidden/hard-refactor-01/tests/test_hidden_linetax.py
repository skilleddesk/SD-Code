import contextlib
import io
import unittest

from shop import cli
from shop.cart import Cart
from shop.orders import place_order
from shop.stock import Stock

STOCK = {"MUG": 50, "TEE": 100, "POSTER": 100, "LAMP": 5}


class LineTaxTest(unittest.TestCase):
    def test_tax_is_summed_per_line_not_taken_on_the_subtotal(self):
        cart = Cart()
        cart.add("MUG", 1)
        cart.add("TEE", 1)
        order = place_order(cart, "US-NY", "A", Stock(STOCK))

        self.assertEqual(order.line_taxes, [75, 177])
        self.assertEqual(order.tax_cents, 252, "the whole-subtotal rule would say 253")

    def test_the_other_direction_rounds_up(self):
        cart = Cart()
        cart.add("TEE", 2)
        cart.add("POSTER", 1)
        order = place_order(cart, "US-NY", "A", Stock(STOCK))

        self.assertEqual(order.line_taxes, [355, 107])
        self.assertEqual(order.tax_cents, 462)

    def test_a_bulk_line_is_taxed_after_its_discount(self):
        cart = Cart()
        cart.add("TEE", 10)
        order = place_order(cart, "EU", "A", Stock(STOCK))

        self.assertEqual(order.subtotal_cents, 17991)
        self.assertEqual(order.line_taxes, [3598])
        self.assertEqual(order.tax_cents, 3598)

    def test_the_cli_prints_a_line_per_cart_line(self):
        out = io.StringIO()

        with contextlib.redirect_stdout(out):
            code = cli.main(["US-NY", "A", "MUG:1", "TEE:1"])

        lines = out.getvalue().splitlines()

        self.assertEqual(code, 0)
        self.assertEqual(lines[0], "  MUG tax $0.75")
        self.assertEqual(lines[1], "  TEE tax $1.77")
        self.assertTrue(lines[2].startswith("subtotal"))
        self.assertTrue(lines[3].startswith("tax"))
        self.assertIn("$2.52", lines[3])
