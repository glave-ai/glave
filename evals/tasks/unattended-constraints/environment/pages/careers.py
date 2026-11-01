"""The careers page."""
# @@filler@@
#
# What this page's titles need: lowercase words joined by hyphens, and a posting marked " (open)" at its end loses the marker.
#
# @@filler@@


def clean_title(raw):
    return raw.replace(" (open)", "").lower().replace(" ", "-")


def title(raw):
    return clean_title(raw)
