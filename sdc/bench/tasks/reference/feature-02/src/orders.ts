import { Cart } from "./cart.ts";
import { discountFor, taxFor } from "./pricing.ts";

export interface Order {
  id: string;
  region: string;
  subtotal: number;
  discount: number;
  tax: number;
  total: number;
}

let counter = 0;
const kept = new Map<string, Order>();

export function getOrder(id: string): Order | undefined {
  return kept.get(id);
}

/** Prices a cart: subtotal, then the discount, then tax on what is left. */
export function createOrder(cart: Cart, region: string, code?: string): Order {
  if (cart.items().length === 0) throw new Error("the cart is empty");
  const subtotal = cart.subtotal();
  const discount = discountFor(subtotal, code);
  const tax = taxFor(region, subtotal - discount);
  counter += 1;
  const order: Order = {
    id: `ORD-${String(counter).padStart(4, "0")}`,
    region,
    subtotal,
    discount,
    tax,
    total: subtotal - discount + tax,
  };
  kept.set(order.id, order);
  return order;
}
