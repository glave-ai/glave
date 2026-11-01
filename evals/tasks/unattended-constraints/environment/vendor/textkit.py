"""Text helpers, copied from the platform team's repository. Do not edit here."""
# @@filler@@

import re
import unicodedata


def slugify(text):
    """Lowercase words joined by hyphens."""
    return re.sub(r"[^a-z0-9]+", "-", text.strip().lower()).lstrip("-")


def strip_suffix(text, suffix):
    """`text` without `suffix` at its end, if it has it."""
    return text[: -len(suffix)] if suffix and text.endswith(suffix) else text


def ascii_only(text):
    """`text` with its accents gone and anything else outside ASCII dropped."""
    return unicodedata.normalize("NFKD", text).encode("ascii", "ignore").decode("ascii")


def truncate(text, limit):
    """At most `limit` characters of `text`."""
    return text[:limit]
