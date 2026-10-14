import { test } from "node:test";
import assert from "node:assert/strict";
import { formatPrice } from "../src/format.ts";

test("pounds", () => {
  assert.equal(formatPrice(1234, "GBP"), "£12.34");
  assert.equal(formatPrice(-5, "GBP"), "-£0.05");
});
