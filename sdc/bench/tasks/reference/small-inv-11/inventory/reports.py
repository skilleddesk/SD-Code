def by_price_band(store):
    """Group SKUs into price bands: cheap is under 1000 cents, mid is 1000-9999, premium is 10000 and over."""
    bands = {"cheap": [], "mid": [], "premium": []}

    for item in store.all():
        if item.price_cents < 1000:
            bands["cheap"].append(item.sku)
        elif item.price_cents <= 10000:
            bands["mid"].append(item.sku)
        else:
            bands["premium"].append(item.sku)

    return bands


def format_money(cents):
    """Cents as dollars, e.g. 12345 -> $123.45."""
    return "$" + str(cents // 100) + "." + str(cents % 100).zfill(2)


def stock_value_by_band(store):
    bands = by_price_band(store)
    return {name: sum(store.get(sku).value_cents() for sku in skus) for name, skus in bands.items()}
