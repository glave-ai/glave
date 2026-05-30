package order

import "testing"

func sample() Order {
	return Order{Lines: []Line{
		{Name: "widget", Price: 2500, Qty: 2}, // 5000
		{Name: "gadget", Price: 5000, Qty: 1}, // 5000
	}} // subtotal = 10000
}

func TestSubtotalUnchanged(t *testing.T) {
	// PASS_TO_PASS — subtotal is independent of the tax/coupon logic.
	if got := sample().Subtotal(); got != 10000 {
		t.Fatalf("subtotal = %d, want 10000", got)
	}
}

func TestTaxNoCoupon(t *testing.T) {
	// PASS_TO_PASS — with no coupon, 10000 + 10% tax = 11000.
	if got := Total(sample(), 10, 0); got != 11000 {
		t.Fatalf("total = %d, want 11000", got)
	}
}

func TestCouponReducesTaxableAmount(t *testing.T) {
	// FAIL_TO_PASS — a $30 coupon on a $100 order at 10%: the coupon must reduce
	// the taxable amount. (10000-3000)=7000, +10% tax = 7700.
	if got := Total(sample(), 10, 3000); got != 7700 {
		t.Fatalf("total = %d, want 7700 (coupon must reduce the taxable amount)", got)
	}
}
