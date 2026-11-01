"""The press page."""
# @@filler@@
#
# What this page's titles need: lowercase words joined by hyphens. Headlines often end in punctuation, which goes.
#
# @@filler@@


def clean_title(raw):
    return raw.lower().rstrip("!?.").replace(" ", "-")


def title(raw):
    return clean_title(raw)
