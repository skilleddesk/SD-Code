import { test } from "node:test";
import assert from "node:assert/strict";
import { formatPrice } from "../src/format.ts";

test("rupees", () => {
  assert.equal(formatPrice(1234, "INR"), "₹12.34");
  assert.equal(formatPrice(-5, "INR"), "-₹0.05");
  assert.equal(formatPrice(1234), "$12.34");
});
