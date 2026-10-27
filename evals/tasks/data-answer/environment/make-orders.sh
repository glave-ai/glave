#!/bin/sh
# Makes orders.csv, a year of orders as a shop's system exports them, and is removed once it
# has (task.toml, setup). The rows come from a fixed sequence (Park and Miller's), in awk's
# plain arithmetic, so every awk makes the same file. Some customers' names hold a comma, so
# their field is quoted, as CSV quotes one; splitting lines on commas reads those rows wrong.
set -eu
awk 'BEGIN {
  seed = 20261027
  split("Acme, Inc.|Brightside Ltd|Smith & Sons|Nakamura, K.|Oak Street Cafe|Patel, R.|Greenway Co", customer, "|")
  split("North|South|East|West", region, "|")
  split("Hardware|Software|Services", category, "|")
  split("49.99 89.5 120", price1, " ")
  split("19 29.99 199", price2, " ")
  split("75 150 240", price3, " ")
  split("0 0.05 0.1 0.15 0.2", discount, " ")
  print "order_id,date,customer,region,category,units,unit_price,discount,returned"
  for (id = 1001; id <= 1360; id++) {
    month = next_in(12); day = next_in(28)
    who = customer[next_in(7)]
    if (index(who, ",")) who = "\"" who "\""
    c = next_in(3)
    units = next_in(40)
    p = next_in(3)
    price = (c == 1) ? price1[p] : (c == 2) ? price2[p] : price3[p]
    # Larger orders tend to get larger discounts.
    d = int((units - 1) / 10) + next_in(2) - 1
    if (d > 4) d = 4
    returned = (next_in(100) <= 8) ? "yes" : "no"
    printf "%d,2026-%02d-%02d,%s,%s,%s,%d,%s,%s,%s\n", id, month, day, who, region[next_in(4)], category[c], units, price, discount[d + 1], returned
  }
}
# The next number in the sequence, as 1 to n.
function next_in(n) {
  seed = (seed * 16807) % 2147483647
  return seed % n + 1
}' > orders.csv
