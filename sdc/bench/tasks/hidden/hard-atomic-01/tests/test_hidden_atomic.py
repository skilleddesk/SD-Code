import unittest

from shop.cart import Cart
from shop.orders import place_order
from shop.stock import OutOfStock, Stock


def cart_of(*items):
    cart = Cart()

    for sku, quantity in items:
        cart.add(sku, quantity)

    return cart


class AtomicTest(unittest.TestCase):
    def test_a_short_line_takes_nothing(self):
        stock = Stock({"TEE": 5, "LAMP": 1})

        with self.assertRaises(OutOfStock) as caught:
            place_order(cart_of(("TEE", 2), ("LAMP", 3)), "EU", "A", stock)

        self.assertEqual(caught.exception.args[0], "LAMP")
        self.assertEqual(stock.levels, {"TEE": 5, "LAMP": 1})

    def test_the_first_short_product_is_the_one_named(self):
        stock = Stock({"TEE": 1, "LAMP": 1})

        with self.assertRaises(OutOfStock) as caught:
            place_order(cart_of(("TEE", 2), ("LAMP", 3)), "EU", "A", stock)

        self.assertEqual(caught.exception.args[0], "TEE")

    def test_an_unknown_zone_takes_nothing(self):
        stock = Stock({"TEE": 5})

        with self.assertRaises(KeyError):
            place_order(cart_of(("TEE", 2)), "EU", "Z", stock)

        self.assertEqual(stock.levels, {"TEE": 5})

    def test_a_product_the_shelf_has_never_heard_of_is_out_of_stock(self):
        stock = Stock({"TEE": 5})

        with self.assertRaises(OutOfStock):
            place_order(cart_of(("TEE", 1), ("MUG", 1)), "EU", "A", stock)

        self.assertEqual(stock.levels, {"TEE": 5})

    def test_a_good_order_still_takes_its_stock(self):
        stock = Stock({"TEE": 5, "LAMP": 2})
        place_order(cart_of(("TEE", 2), ("LAMP", 1)), "EU", "A", stock)

        self.assertEqual(stock.levels, {"TEE": 3, "LAMP": 1})
