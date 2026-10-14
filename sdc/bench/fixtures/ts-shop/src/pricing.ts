// Discount codes and sales tax. All amounts are integer cents.

interface Code {
  kind: "percent" | "flat";
  value: number;
  minOrderCents: number;
}

export const CODES: Record<string, Code> = {
  SAVE10: { kind: "percent", value: 10, minOrderCents: 0 },
  FLAT5: { kind: "flat", value: 500, minOrderCents: 2000 },
};

/** The discount a code gives on a subtotal, never more than the subtotal. Unknown codes give 0. */
export function discountFor(subtotalCents: number, code?: string): number {
  if (!code) return 0;
  const rule = CODES[code.toUpperCase()];
  if (!rule || subtotalCents <= rule.minOrderCents) return 0;
  const amount = rule.kind === "percent" ? Math.round((subtotalCents * rule.value) / 100) : rule.value;
  return Math.min(amount, subtotalCents);
}

const TAX_RATES: Record<string, number> = { US: 0.07, DE: 0.19, BD: 0.15 };

/** Sales tax on an amount for a region, rounded to the cent. */
export function taxFor(region: string, amountCents: number): number {
  const rate = TAX_RATES[region];
  if (rate === undefined) throw new Error(`no tax rate for ${region}`);
  return Math.round(amountCents * rate);
}
