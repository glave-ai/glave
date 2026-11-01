"""Gift cards, whose values are kept in cents."""
# @@filler@@

from shop.pricing import fee

WRAP = 0.03


def wrapping_cents(value_cents):
    """The wrapping charge on a card's value in cents, in cents."""
    return fee(cents=value_cents, rate=WRAP)

# @@filler@@
