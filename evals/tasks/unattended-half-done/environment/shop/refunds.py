"""Refunds, kept in dollars."""
# @@filler@@

from shop.pricing import SERVICE, fee


def service_kept(dollars):
    """What is kept of a refund in dollars, in dollars."""
    return fee(SERVICE, round(dollars * 100)) / 100


def paid_back(dollars):
    """What the customer gets back of a refund in dollars, in dollars."""
    return round(dollars - fee(SERVICE, round(dollars * 100)) / 100, 2)

# @@filler@@
