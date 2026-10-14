// Prices are integer cents everywhere; this is the only place they become text.

const SYMBOLS: Record<string, { symbol: string; after: boolean; decimal: string }> = {
  USD: { symbol: "$", after: false, decimal: "." },
  EUR: { symbol: " €", after: true, decimal: "," },
  BDT: { symbol: "৳", after: false, decimal: "." },
};

export function formatPrice(cents: number, currency = "USD"): string {
  const style = SYMBOLS[currency];
  if (!style) throw new Error(`unknown currency ${currency}`);
  const sign = cents < 0 ? "-" : "";
  const abs = Math.abs(cents);
  const text = `${Math.floor(abs / 100)}${style.decimal}${String(abs % 100).padStart(2, "0")}`;
  return style.after ? `${sign}${text}${style.symbol}` : `${sign}${style.symbol}${text}`;
}
