"""Bundles, priced from a table, in cents."""
# @@filler@@

from shop import pricing

RATES = {"standard": 0.1, "large": 0.15}
CHARGES = {"plain": pricing.fee}


def extra_cents(kind, cents):
    """The extra charge on a bundle of this kind, in cents."""
    charge = CHARGES["plain"]
    return charge(RATES[kind], cents)

# @@filler@@
