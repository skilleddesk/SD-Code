import { test } from "node:test";
import assert from "node:assert/strict";
import { Inventory } from "../src/inventory.ts";

test("the last unit can be reserved", () => {
  const stock = new Inventory({ tea: 2 });
  stock.reserve("tea", 2);
  assert.equal(stock.available("tea"), 0);
  assert.throws(() => stock.reserve("tea", 1));
});
