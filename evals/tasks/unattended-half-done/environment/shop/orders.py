"""Orders, whose totals are kept in cents."""
# @@filler@@

from shop.pricing import VAT, fee


def total_cents(subtotal_cents):
    """The order's total with VAT, in cents."""
    return subtotal_cents + fee(VAT, subtotal_cents)


def vat_line(subtotal_cents):
    """The invoice line for VAT, in cents, never below one cent."""
    return max(
        fee(
            VAT,
            subtotal_cents,
        ),
        1,
    )

# @@filler@@
