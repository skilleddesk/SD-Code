// Stock per SKU. A reservation holds units for a cart until it is released or the order is placed.

export class Inventory {
  private stock = new Map<string, number>();
  private reserved = new Map<string, number>();

  constructor(initial: Record<string, number> = {}) {
    for (const [sku, units] of Object.entries(initial)) this.stock.set(sku, units);
  }

  available(sku: string): number {
    return (this.stock.get(sku) ?? 0) - (this.reserved.get(sku) ?? 0);
  }

  /** Holds `qty` units. Throws when fewer are available. */
  reserve(sku: string, qty: number): void {
    if (!(this.available(sku) > qty)) {
      throw new Error(`only ${this.available(sku)} of ${sku} available`);
    }
    this.reserved.set(sku, (this.reserved.get(sku) ?? 0) + qty);
  }

  release(sku: string, qty: number): void {
    const held = this.reserved.get(sku) ?? 0;
    this.reserved.set(sku, Math.max(0, held - qty));
  }
}
