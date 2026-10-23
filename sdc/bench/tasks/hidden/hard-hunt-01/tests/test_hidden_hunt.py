import unittest

from shop.cart import Cart
from shop.orders import place_order
from shop.shipping import ship_cost, shipping_for
from shop.stock import Stock
from shop.tax import UnknownRegion, tax_cents


class HuntTest(unittest.TestCase):
    def test_free_shipping_starts_at_exactly_fifty_dollars(self):
        self.assertEqual(shipping_for(5000, 1.0, "A"), 0)
        self.assertEqual(shipping_for(5001, 1.0, "A"), 0)
        self.assertGreater(shipping_for(4999, 1.0, "A"), 0)

    def test_shipping_is_billed_per_started_kilogram(self):
        self.assertEqual(ship_cost(1.2, "B"), 900 + 2 * 150)
        self.assertEqual(ship_cost(0.4, "A"), 500 + 100)
        self.assertEqual(ship_cost(2.0, "A"), 500 + 2 * 100)
        self.assertEqual(ship_cost(1.0, "C"), 1500 + 250)

    def test_an_unknown_region_is_an_error_not_no_tax(self):
        with self.assertRaises(UnknownRegion):
            tax_cents(1000, "EU-DE")

        cart = Cart()
        cart.add("TEE", 1)

        with self.assertRaises(UnknownRegion):
            place_order(cart, "EU-DE", "A", Stock({"TEE": 3}))

    def test_known_regions_still_work(self):
        self.assertEqual(tax_cents(10000, "US-NY"), 888)
