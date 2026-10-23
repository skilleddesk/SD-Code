import unittest

from shop.cart import Cart
from shop.money import fmt, percent_of
from shop.orders import cancel_order, place_order
from shop.pricing import bulk_discount_percent
from shop.shipping import ship_cost
from shop.stock import OutOfStock, Stock
from shop.tax import tax_cents


class ShopTest(unittest.TestCase):
    def test_money(self):
        self.assertEqual(percent_of(1000, 7.5), 75)
        self.assertEqual(percent_of(199, 10), 20)
        self.assertEqual(fmt(1999), "$19.99")
        self.assertEqual(fmt(-5), "-$0.05")

    def test_bulk_discount(self):
        self.assertEqual(bulk_discount_percent(9), 0)
        self.assertEqual(bulk_discount_percent(12), 10)

    def test_tax(self):
        self.assertEqual(tax_cents(10000, "US-CA"), 725)
        self.assertEqual(tax_cents(10000, "EU"), 2000)

    def test_shipping_whole_kilograms(self):
        self.assertEqual(ship_cost(2.0, "A"), 700)
        self.assertEqual(ship_cost(1.0, "B"), 1050)

    def test_an_order_is_priced_and_takes_its_stock(self):
        cart = Cart()
        cart.add("TEE", 2)
        cart.add("POSTER", 1)
        stock = Stock({"TEE": 5, "POSTER": 5})
        order = place_order(cart, "US-CA", "A", stock)

        self.assertEqual(order.subtotal_cents, 1999 * 2 + 1200)
        self.assertEqual(order.tax_cents, percent_of(order.subtotal_cents, 7.25))
        self.assertEqual(order.shipping_cents, 0, "5198 is over the free-shipping line")
        self.assertEqual(stock.levels, {"TEE": 3, "POSTER": 4})

    def test_out_of_stock(self):
        cart = Cart()
        cart.add("LAMP", 3)

        with self.assertRaises(OutOfStock):
            place_order(cart, "EU", "A", Stock({"LAMP": 1}))

    def test_cancel_marks_the_order(self):
        cart = Cart()
        cart.add("MUG", 1)
        order = place_order(cart, "EU", "C", Stock({"MUG": 3}))
        cancel_order(order, Stock({"MUG": 3}))

        self.assertEqual(order.status, "cancelled")


if __name__ == "__main__":
    unittest.main()
