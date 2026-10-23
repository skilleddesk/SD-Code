import unittest

from shop.cart import Cart
from shop.orders import place_order
from shop.shipping import Parcel, ship_cost, shipping_for
from shop.stock import Stock


class ParcelTest(unittest.TestCase):
    def test_a_parcel_is_priced_by_zone_and_weight(self):
        self.assertEqual(ship_cost(Parcel(2.0, "A")), 500 + 2 * 100)
        self.assertEqual(ship_cost(Parcel(1.0, "B")), 900 + 150)

    def test_a_fragile_parcel_costs_two_dollars_more(self):
        self.assertEqual(ship_cost(Parcel(2.0, "A", fragile=True)), 500 + 2 * 100 + 200)

    def test_free_shipping_still_wins(self):
        self.assertEqual(shipping_for(9000, Parcel(2.0, "A", fragile=True)), 0)
        self.assertEqual(shipping_for(1000, Parcel(2.0, "A")), 700)

    def test_a_parcel_is_a_value(self):
        self.assertEqual(Parcel(1.0, "A"), Parcel(1.0, "A"))

        with self.assertRaises(Exception):
            Parcel(1.0, "A").weight_kg = 2.0

    def test_an_order_with_a_fragile_product_ships_as_fragile(self):
        fragile = Cart()
        fragile.add("MUG", 1)
        plain = Cart()
        plain.add("POSTER", 1)
        stock = Stock({"MUG": 5, "POSTER": 5})

        self.assertEqual(place_order(fragile, "EU", "B", stock).shipping_cents, ship_cost(Parcel(fragile.weight_kg(), "B", fragile=True)))
        self.assertEqual(place_order(plain, "EU", "B", stock).shipping_cents, ship_cost(Parcel(plain.weight_kg(), "B", fragile=False)))
