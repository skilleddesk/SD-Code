from dataclasses import dataclass


@dataclass(frozen=True)
class Product:
    sku: str
    name: str
    price_cents: int
    weight_kg: float
    fragile: bool = False


PRODUCTS = {
    product.sku: product
    for product in [
        Product("MUG", "Mug", 850, 0.4, True),
        Product("TEE", "T-shirt", 1999, 0.2),
        Product("POSTER", "Poster", 1200, 0.1),
        Product("LAMP", "Desk lamp", 4500, 1.8, True),
    ]
}


def get(sku: str) -> Product:
    return PRODUCTS[sku]
