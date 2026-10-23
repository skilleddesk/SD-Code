import unittest

from shop.cart import Cart
from shop.orders import OrderStateError, cancel_order, place_order
from shop.stock import Stock


class CancelTest(unittest.TestCase):
    def test_cancelling_gives_the_stock_back(self):
        cart = Cart()
        cart.add("MUG", 2)
        cart.add("TEE", 1)
        stock = Stock({"MUG": 10, "TEE": 4})
        order = place_order(cart, "EU", "A", stock)

        self.assertEqual(stock.levels, {"MUG": 8, "TEE": 3})

        cancel_order(order, stock)

        self.assertEqual(order.status, "cancelled")
        self.assertEqual(stock.levels, {"MUG": 10, "TEE": 4})

    def test_a_second_cancel_is_an_error_and_returns_nothing(self):
        cart = Cart()
        cart.add("MUG", 2)
        stock = Stock({"MUG": 10})
        order = place_order(cart, "EU", "A", stock)

        cancel_order(order, stock)

        with self.assertRaises(OrderStateError):
            cancel_order(order, stock)

        self.assertEqual(stock.levels, {"MUG": 10})
