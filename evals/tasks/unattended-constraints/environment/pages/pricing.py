"""The pricing page."""
# @@filler@@
#
# What this page's titles need: lowercase words joined by hyphens, with the punctuation gone: "Plans from $5!" is plans-from-5.
#
# @@filler@@


def clean_title(raw):
    return "-".join(raw.lower().replace("$", "").rstrip("!").split())


def title(raw):
    return clean_title(raw)
