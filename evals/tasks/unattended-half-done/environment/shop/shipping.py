"""Shipping, whose charges are kept in cents."""
# @@filler@@

from shop.pricing import fee as levy

FUEL = 0.1


def charge_cents(distance_km):
    """The charge for a distance in kilometres, with fuel, in cents."""
    base = 200 + 3 * distance_km
    return base + levy(FUEL, base)

# @@filler@@
