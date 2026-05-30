def median(values):
    """Return the median of a list of numbers.

    BUG: returns the upper-middle element for even-length lists instead of
    averaging the two middle elements.
    """
    s = sorted(values)
    n = len(s)
    return s[n // 2]
