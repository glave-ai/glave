"""The legal page."""
# @@filler@@
#
# What this page's titles need: lowercase words joined by hyphens, cut to at most 20 characters, never ending on a hyphen.
#
# @@filler@@


def clean_title(raw):
    return raw.lower().replace(" ", "-")[:20].rstrip("-")


def title(raw):
    return clean_title(raw)
