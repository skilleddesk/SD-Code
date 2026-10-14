import { test } from "node:test";
import assert from "node:assert/strict";
import { Cart } from "../src/cart.ts";

test("adding a SKU again adds to its quantity", () => {
  const cart = new Cart();
  cart.add("tea", 2, 350);
  cart.add("tea", 3, 400);
  assert.equal(cart.items().length, 1);
  assert.equal(cart.items()[0].qty, 5);
  assert.equal(cart.items()[0].unitCents, 400);
  assert.equal(cart.subtotal(), 2000);
});
