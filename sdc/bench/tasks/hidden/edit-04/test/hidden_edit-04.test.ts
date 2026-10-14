import { test } from "node:test";
import assert from "node:assert/strict";
import { Cart } from "../src/cart.ts";

test("clear empties the cart", () => {
  const cart = new Cart();
  cart.add("tea", 1, 100);
  cart.add("mug", 2, 900);
  cart.clear();
  assert.equal(cart.items().length, 0);
  assert.equal(cart.subtotal(), 0);
});
