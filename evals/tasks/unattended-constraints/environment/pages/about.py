"""The about page."""
# @@filler@@
#
# What this page's titles need: lowercase words joined by hyphens, however much space the title has.
#
# @@filler@@


def clean_title(raw):
    return "-".join(raw.lower().split())


def title(raw):
    return clean_title(raw)
