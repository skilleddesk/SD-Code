import { test } from "node:test";
import assert from "node:assert/strict";
import { Cart } from "../src/cart.ts";
import { createOrder, getOrder } from "../src/orders.ts";
import { route } from "../src/api/routes.ts";

test("orders are kept and served", () => {
  const cart = new Cart();
  cart.add("tea", 1, 1000);
  const order = createOrder(cart, "US");
  assert.deepEqual(getOrder(order.id), order);
  const found = route("GET", `/orders/${order.id}`);
  assert.ok(found, "GET /orders/:id is routed");
  const answer = found.handler({ params: found.params, body: {} });
  assert.equal(answer.status, 200);
  assert.deepEqual(answer.body, order);
  const missing = route("GET", "/orders/ORD-9999")!;
  assert.equal(missing.handler({ params: missing.params, body: {} }).status, 404);
});
