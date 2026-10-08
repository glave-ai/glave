#!/usr/bin/env bash
set -euo pipefail
perl -pi -e 's/calc_total/order_total/g' README.md src/*.rs tests/*.rs
perl -pi -e 's/order_total\(items: &\[Item\]\)/order_total(order: &Order)/; s/^    items\.iter\(\)/    order.items.iter()/; s/total of `items`/total of an order/' src/order.rs
perl -pi -e 's/order_total\(&self\.order\.items\)/order_total(&self.order)/; s/order_total\(&order\.items\)/order_total(order)/' src/*.rs
perl -pi -e 's/order_total\(&order\(\)\.items\)/order_total(&order())/; s/order_total\(&\[\]\)/order_total(&Order::default())/' tests/shop.rs
