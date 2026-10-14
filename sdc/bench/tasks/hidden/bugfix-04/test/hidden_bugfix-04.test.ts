import { test } from "node:test";
import assert from "node:assert/strict";
import { discountFor } from "../src/pricing.ts";

test("FLAT5 from exactly $20", () => {
  assert.equal(discountFor(2000, "FLAT5"), 500);
  assert.equal(discountFor(1999, "FLAT5"), 0);
  assert.equal(discountFor(2500, "SAVE10"), 250);
});
