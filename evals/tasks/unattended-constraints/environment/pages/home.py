"""The home page."""
# @@filler@@
#
# What this page's titles need: lowercase words joined by hyphens, with accented letters written without their accents.
#
# @@filler@@


def clean_title(raw):
    return raw.lower().replace("é", "e").replace("ü", "u").replace(" ", "-")


def title(raw):
    return clean_title(raw)
