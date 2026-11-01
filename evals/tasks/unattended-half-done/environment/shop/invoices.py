"""Invoices, whose lines are kept in cents."""
# @@filler@@

from functools import partial

from shop.pricing import VAT, fee

vat_on = partial(fee, VAT)


def line_totals(lines):
    """Each line in cents with its VAT, in cents."""
    return [line + vat_on(line) for line in lines]

# @@filler@@
