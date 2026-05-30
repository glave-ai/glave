package order

import "task/money"

// Total computes the amount due: the subtotal, less a fixed-amount coupon, plus
// sales tax. A coupon should reduce the amount that tax is charged on.
func Total(o Order, taxPct int, coupon money.Cents) money.Cents {
	sub := o.Subtotal()
	// BUG: tax is charged on the full subtotal and the coupon is subtracted
	// afterward, so the customer pays tax on amounts the coupon removed.
	tax := money.Percent(sub, taxPct)
	return sub + tax - coupon
}
