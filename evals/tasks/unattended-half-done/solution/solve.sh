#!/usr/bin/env bash
set -euo pipefail
python3 - <<'EOF'
def change(path, *pairs):
    text = open(path, encoding="utf-8").read()
    for old, new in pairs:
        assert old in text, old
        text = text.replace(old, new)
    open(path, "w", encoding="utf-8").write(text)

change(
    "shop/pricing.py",
    ('''def fee(rate, cents):
    """The fee at `rate` on an amount in cents, in whole cents."""
    return round(cents * rate)''',
     '''def fee(dollars, rate):
    """The fee at `rate` on an amount in dollars, in dollars to two places."""
    return round(dollars * rate, 2)

def in_cents(dollars):
    return round(dollars * 100)'''),
)
change(
    "shop/orders.py",
    ("import VAT, fee", "import VAT, fee, in_cents"),
    ("subtotal_cents + fee(VAT, subtotal_cents)", "subtotal_cents + in_cents(fee(subtotal_cents / 100, VAT))"),
    ("""max(
        fee(
            VAT,
            subtotal_cents,
        ),
        1,
    )""", "max(in_cents(fee(subtotal_cents / 100, VAT)), 1)"),
)
change(
    "shop/refunds.py",
    ("return fee(SERVICE, round(dollars * 100)) / 100", "return fee(dollars, SERVICE)"),
    ("dollars - fee(SERVICE, round(dollars * 100)) / 100", "dollars - fee(dollars, SERVICE)"),
)
change(
    "shop/shipping.py",
    ("import fee as levy", "import fee as levy, in_cents"),
    ("base + levy(FUEL, base)", "base + in_cents(levy(base / 100, FUEL))"),
)
change(
    "shop/gifts.py",
    ("import fee", "import fee, in_cents"),
    ("fee(cents=value_cents, rate=WRAP)", "in_cents(fee(dollars=value_cents / 100, rate=WRAP))"),
)
change(
    "shop/bundles.py",
    ("charge(RATES[kind], cents)", "pricing.in_cents(charge(cents / 100, RATES[kind]))"),
)
change(
    "shop/invoices.py",
    ("import VAT, fee", "import VAT, fee, in_cents"),
    ("partial(fee, VAT)", "partial(fee, rate=VAT)"),
    ("line + vat_on(line)", "line + in_cents(vat_on(line / 100))"),
)
EOF
