package order

import "task/money"

// Total computes the amount due: the subtotal, less a fixed-amount coupon, plus
// sales tax. A coupon reduces the amount that tax is charged on.
func Total(o Order, taxPct int, coupon money.Cents) money.Cents {
	sub := o.Subtotal()
	taxable := sub - coupon
	tax := money.Percent(taxable, taxPct)
	return taxable + tax
}
