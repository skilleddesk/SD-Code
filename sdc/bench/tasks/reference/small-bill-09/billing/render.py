def money(cents):
    return "{}.{:02d}".format(cents // 100, cents % 100)


def render_invoice(invoice):
    rows = ["Invoice for " + invoice.customer]

    for line in invoice.lines:
        rows.append("{} x{} @ {}".format(line["description"], line["quantity"], money(line["unit_cents"])))

    rows.append("Subtotal " + money(invoice.subtotal_cents()))
    if invoice.tax_percent > 0:
        percent = ("%g" % invoice.tax_percent)
        rows.append("Tax (" + percent + "%) " + money(invoice.tax_cents()))
    else:
        rows.append("Tax " + money(invoice.tax_cents()))
    rows.append("Total " + money(invoice.total_cents()))
    return "\n".join(rows)
