import datetime
import unittest

from shop.cart import Cart
from shop.coupons import COUPONS, Coupon, InvalidCoupon, discount_cents
from shop.orders import place_order
from shop.shipping import ship_cost
from shop.stock import Stock

TODAY = datetime.date(2026, 10, 1)


def cart_of(*items):
    cart = Cart()

    for sku, quantity in items:
        cart.add(sku, quantity)

    return cart


class CouponTest(unittest.TestCase):
    def test_the_three_coupons(self):
        self.assertEqual(set(COUPONS), {"WELCOME10", "BIG20", "OLD5"})
        self.assertEqual(COUPONS["BIG20"], Coupon("BIG20", 20, 10000, None))
        self.assertEqual(COUPONS["OLD5"].expires, datetime.date(2020, 1, 1))

    def test_a_discount_is_a_rounded_percent_of_the_subtotal(self):
        self.assertEqual(discount_cents("WELCOME10", 1999, TODAY), 200)
        self.assertEqual(discount_cents("BIG20", 10000, TODAY), 2000)

    def test_invalid_coupons(self):
        with self.assertRaises(InvalidCoupon):
            discount_cents("NOPE", 5000, TODAY)

        with self.assertRaises(InvalidCoupon):
            discount_cents("BIG20", 9999, TODAY)

        with self.assertRaises(InvalidCoupon):
            discount_cents("OLD5", 5000, TODAY)

        self.assertEqual(discount_cents("OLD5", 5000, datetime.date(2020, 1, 1)), 250)

    def test_an_order_with_a_coupon(self):
        cart = cart_of(("TEE", 2))
        order = place_order(cart, "EU", "A", Stock({"TEE": 9}), coupon="WELCOME10", today=TODAY)

        self.assertEqual(order.subtotal_cents, 3998)
        self.assertEqual(order.coupon, "WELCOME10")
        self.assertEqual(order.discount_cents, 400)
        self.assertEqual(order.tax_cents, 720)
        self.assertEqual(order.shipping_cents, ship_cost(cart.weight_kg(), "A"))
        self.assertEqual(order.total_cents, 3998 - 400 + 720 + order.shipping_cents)

    def test_free_shipping_looks_at_the_subtotal_after_the_coupon(self):
        stock = {"TEE": 9, "POSTER": 9}
        cart = cart_of(("TEE", 2), ("POSTER", 1))
        without = place_order(cart, "EU", "A", Stock(stock), today=TODAY)
        with_coupon = place_order(cart, "EU", "A", Stock(stock), coupon="WELCOME10", today=TODAY)

        self.assertEqual(without.subtotal_cents, 5198)
        self.assertEqual(without.shipping_cents, 0, "5198 on its own is over the free-shipping line")
        self.assertEqual(with_coupon.discount_cents, 520)
        self.assertEqual(with_coupon.shipping_cents, ship_cost(cart.weight_kg(), "A"), "5198 - 520 is below the line")
        self.assertEqual(with_coupon.tax_cents, 936, "tax is on 4678")

    def test_no_coupon_changes_nothing(self):
        order = place_order(cart_of(("TEE", 2)), "EU", "A", Stock({"TEE": 9}))

        self.assertIsNone(order.coupon)
        self.assertEqual(order.discount_cents, 0)
        self.assertEqual(order.total_cents, order.subtotal_cents + order.tax_cents + order.shipping_cents)

    def test_an_invalid_coupon_takes_no_stock(self):
        stock = Stock({"TEE": 9})

        with self.assertRaises(InvalidCoupon):
            place_order(cart_of(("TEE", 2)), "EU", "A", stock, coupon="NOPE", today=TODAY)

        self.assertEqual(stock.levels, {"TEE": 9})
