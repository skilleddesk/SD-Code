import unittest

from shop.cart import Cart
from shop.orders import place_order
from shop.stock import Stock


class MergeTest(unittest.TestCase):
    def test_the_same_sku_adds_to_its_line(self):
        cart = Cart()
        cart.add("MUG", 6)
        cart.add("TEE", 1)
        cart.add("MUG", 4)

        self.assertEqual(cart.items, [["MUG", 10], ["TEE", 1]])

    def test_ten_units_in_two_adds_get_the_bulk_discount(self):
        cart = Cart()
        cart.add("MUG", 6)
        cart.add("MUG", 4)
        order = place_order(cart, "EU", "A", Stock({"MUG": 20}))

        self.assertEqual(order.subtotal_cents, 8500 - 850)
        self.assertEqual(order.lines, [("MUG", 10)])

    def test_stock_is_taken_once_for_the_merged_line(self):
        cart = Cart()
        cart.add("MUG", 6)
        cart.add("MUG", 4)
        stock = Stock({"MUG": 12})
        place_order(cart, "EU", "A", stock)

        self.assertEqual(stock.levels["MUG"], 2)

    def test_a_quantity_must_be_positive(self):
        cart = Cart()

        for bad in (0, -1):
            with self.assertRaises(ValueError):
                cart.add("MUG", bad)

        self.assertEqual(cart.items, [])
