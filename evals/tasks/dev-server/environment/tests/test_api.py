"""Integration tests: they need the server running (python3 server.py)."""

import json
import pathlib
import unittest
import urllib.request


def get(path):
    address = pathlib.Path(".server-address").read_text().strip()
    with urllib.request.urlopen(address + path, timeout=5) as response:
        return json.load(response)


class TotalTest(unittest.TestCase):
    def test_adds_every_price(self):
        self.assertEqual(get("/total?prices=1.50,2.25,0.25"), {"total": "4.00"})

    def test_one_price(self):
        self.assertEqual(get("/total?prices=3.10"), {"total": "3.10"})

    def test_no_prices(self):
        self.assertEqual(get("/total?prices="), {"total": "0"})


if __name__ == "__main__":
    unittest.main()
