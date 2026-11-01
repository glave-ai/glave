"""What things cost on top of their price."""
# @@filler@@

VAT = 0.2
SERVICE = 0.05


def fee(rate, cents):
    """The fee at `rate` on an amount in cents, in whole cents."""
    return round(cents * rate)

# @@filler@@
