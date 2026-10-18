from .money import percent_of


class Invoice:
    def __init__(self, customer, tax_percent=0):
        self.customer = customer
        self.tax_percent = tax_percent
        self.lines = []

    def add_line(self, description, unit_cents, quantity=1):
        if quantity <= 0:
            raise ValueError("quantity must be positive")
        self.lines.append({"description": description, "unit_cents": unit_cents, "quantity": quantity})

    def remove_line(self, description):
        before = len(self.lines)
        self.lines = [line for line in self.lines if line["description"] != description]
        return before - len(self.lines)

    def subtotal_cents(self):
        return sum(line["unit_cents"] * line["quantity"] for line in self.lines)

    def tax_cents(self):
        return percent_of(self.subtotal_cents(), self.tax_percent)

    def total_cents(self):
        return self.subtotal_cents() + self.tax_cents()
