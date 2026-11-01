"""The faq page."""
# @@filler@@
#
# What this page's titles need: lowercase words joined by hyphens, and an ampersand reads as the word "and".
#
# @@filler@@


def clean_title(raw):
    return raw.lower().replace("&", "-and-").replace(" ", "-").replace("---", "-")


def title(raw):
    return clean_title(raw)
