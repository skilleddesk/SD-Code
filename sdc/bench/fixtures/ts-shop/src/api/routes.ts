import { Cart } from "../cart.ts";
import { createOrder, type Order } from "../orders.ts";

export interface Request {
  params: Record<string, string>;
  body: Record<string, unknown>;
}

export interface Response {
  status: number;
  body: unknown;
}

const cart = new Cart();

export function getCartHandler(_req: Request): Response {
  return { status: 200, body: { items: cart.items(), subtotal: cart.subtotal() } };
}

export function addItemHandler(req: Request): Response {
  const { sku, qty, unitCents } = req.body as { sku: string; qty: number; unitCents: number };
  cart.add(sku, qty, unitCents);
  return { status: 201, body: { items: cart.items() } };
}

export function removeItemHandler(req: Request): Response {
  return cart.remove(req.params.sku) ? { status: 204, body: null } : { status: 404, body: { error: "not in cart" } };
}

export function createOrderHandler(req: Request): Response {
  const order: Order = createOrder(cart, String(req.body.region ?? "US"), req.body.code as string | undefined);
  return { status: 201, body: order };
}

export const ROUTES: Array<[method: string, path: string, handler: (req: Request) => Response]> = [
  ["GET", "/cart", getCartHandler],
  ["POST", "/cart/items", addItemHandler],
  ["DELETE", "/cart/items/:sku", removeItemHandler],
  ["POST", "/orders", createOrderHandler],
];

/** Finds the handler for a method and path, filling `:name` parameters. */
export function route(method: string, path: string): { handler: (req: Request) => Response; params: Record<string, string> } | undefined {
  for (const [m, pattern, handler] of ROUTES) {
    if (m !== method) continue;
    const want = pattern.split("/");
    const got = path.split("/");
    if (want.length !== got.length) continue;
    const params: Record<string, string> = {};
    if (want.every((part, i) => (part.startsWith(":") ? ((params[part.slice(1)] = got[i]), true) : part === got[i]))) {
      return { handler, params };
    }
  }
  return undefined;
}
