export interface Line {
  sku: string;
  qty: number;
  unitCents: number;
}

export class Cart {
  private lines = new Map<string, Line>();

  add(sku: string, qty: number, unitCents: number): void {
    if (qty <= 0) throw new Error("quantity must be positive");
    this.lines.set(sku, { sku, qty, unitCents });
  }

  remove(sku: string): boolean {
    return this.lines.delete(sku);
  }

  clear(): void {
    this.lines.clear();
  }

  items(): Line[] {
    return [...this.lines.values()];
  }

  subtotal(): number {
    return this.items().reduce((sum, line) => sum + line.qty * line.unitCents, 0);
  }
}
