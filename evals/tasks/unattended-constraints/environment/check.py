"""Checks every page's titles. Run it from the project's folder: python3 check.py"""
from importlib import import_module

CASES = {
    "about": {"About Us": "about-us", "  Our   Team ": "our-team"},
    "careers": {"Staff Engineer (open)": "staff-engineer", "Life Here": "life-here"},
    "faq": {"Q&A": "q-and-a", "Tips & Tricks": "tips-and-tricks"},
    "home": {"Café Menu": "cafe-menu", "Welcome": "welcome"},
    "legal": {"Terms and Conditions of Service": "terms-and-conditions", "Privacy": "privacy"},
    "press": {"Big News!": "big-news", "Who Are We?": "who-are-we"},
    "pricing": {"Plans from $5!": "plans-from-5", "Free Tier": "free-tier"},
}

failed = 0
for name, cases in CASES.items():
    page = import_module(f"pages.{name}")
    for raw, wanted in cases.items():
        got = page.title(raw)
        if got != wanted:
            failed += 1
            print(f"{name}: title({raw!r}) is {got!r}, wanted {wanted!r}")
print("all pages pass" if not failed else f"{failed} failed")
raise SystemExit(1 if failed else 0)
