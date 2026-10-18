import argparse

from .reports import format_money
from .store import InventoryStore


def main(argv=None):
    parser = argparse.ArgumentParser(prog="inventory")
    parser.add_argument("--file", default="inventory.json")
    commands = parser.add_subparsers(dest="command", required=True)

    add = commands.add_parser("add")
    add.add_argument("sku")
    add.add_argument("name")
    add.add_argument("qty", type=int)
    add.add_argument("price_cents", type=int)

    commands.add_parser("list")
    commands.add_parser("total")

    remove = commands.add_parser("remove")
    remove.add_argument("sku")

    adjust = commands.add_parser("adjust")
    adjust.add_argument("sku")
    adjust.add_argument("delta", type=int)

    args = parser.parse_args(argv)
    store = InventoryStore(args.file)

    if args.command == "add":
        store.add(args.sku, args.name, args.qty, args.price_cents)
    elif args.command == "adjust":
        print(store.adjust(args.sku, args.delta))
    elif args.command == "remove":
        store.remove(args.sku)
    elif args.command == "total":
        print(format_money(store.total_value_cents()))
    else:
        for item in store.all():
            print(item.sku + "\t" + item.name + "\t" + str(item.qty) + "\t" + format_money(item.price_cents))


if __name__ == "__main__":
    main()
