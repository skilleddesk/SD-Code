# py-shop

A tiny shop: a catalogue, a cart, prices, tax, shipping, stock and orders. Money is always whole cents.

Rules the shop follows:

* Percent amounts (discounts, tax) are rounded half up, in cents (`shop.money.percent_of`).
* A line of 10 or more units of one product gets 10 % off that line.
* Tax is charged by region; a region the shop does not know is an error, never "no tax".
* Shipping is billed per **started** kilogram on top of the zone's base price, and is free from a subtotal of 50.00 (5000 cents) **inclusive**.
* An order takes its stock when it is placed and gives it back when it is cancelled.
