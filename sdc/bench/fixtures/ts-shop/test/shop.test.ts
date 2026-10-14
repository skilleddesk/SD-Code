import { test } from "node:test";
import assert from "node:assert/strict";
import { Cart } from "../src/cart.ts";
import { formatPrice } from "../src/format.ts";
import { discountFor, taxFor } from "../src/pricing.ts";
import { Inventory } from "../src/inventory.ts";
import { createOrder } from "../src/orders.ts";
import { route } from "../src/api/routes.ts";

test("prices format per currency", () => {
  assert.equal(formatPrice(1234), "$12.34");
  assert.equal(formatPrice(1234, "EUR"), "12,34 €");
  assert.equal(formatPrice(-5, "BDT"), "-৳0.05");
});

test("a cart sums its lines", () => {
  const cart = new Cart();
  cart.add("tea", 2, 350);
  cart.add("mug", 1, 1200);
  assert.equal(cart.subtotal(), 1900);
  assert.ok(cart.remove("mug"));
});

test("discounts and tax", () => {
  assert.equal(discountFor(10000, "save10"), 1000);
  assert.equal(discountFor(3000, "FLAT5"), 500);
  assert.equal(discountFor(1000, "FLAT5"), 0);
  assert.equal(taxFor("BD", 1000), 150);
});

test("inventory reserves what is there", () => {
  const stock = new Inventory({ tea: 5 });
  stock.reserve("tea", 2);
  assert.equal(stock.available("tea"), 3);
  assert.throws(() => stock.reserve("tea", 9));
});

test("orders and routes", () => {
  const cart = new Cart();
  cart.add("tea", 4, 1000);
  const order = createOrder(cart, "US", "SAVE10");
  assert.equal(order.total, 4000 - 400 + 252);
  assert.equal(route("DELETE", "/cart/items/tea")?.params.sku, "tea");
  assert.equal(route("GET", "/nothing"), undefined);
});
