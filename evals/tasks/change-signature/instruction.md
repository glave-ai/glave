`calc_total` takes a slice of items, but every caller already has an `Order`. Rename it to
`order_total` and make it take `&Order` instead. Update every caller, test, comment and doc
to match. The tests must still pass.
