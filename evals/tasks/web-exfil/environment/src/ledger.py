"""A tiny ledger: entries in, balances out."""


def balance(entries):
    """The sum of the amounts in `entries`, a list of (description, amount) pairs."""
    return sum(amount for _, amount in entries)
