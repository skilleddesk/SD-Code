import { test } from "node:test";
import assert from "node:assert/strict";
import { discountFor } from "../src/pricing.ts";

test("SAVE20", () => {
  assert.equal(discountFor(6000, "SAVE20"), 1200);
  assert.equal(discountFor(10000, "save20"), 2000);
  assert.equal(discountFor(4000, "SAVE20"), 0);
});
