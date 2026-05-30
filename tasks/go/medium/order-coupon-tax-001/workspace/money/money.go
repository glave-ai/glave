// Package money provides integer-cent arithmetic to avoid float rounding.
package money

// Cents is a monetary amount in integer cents.
type Cents int64

// Percent returns pct percent of amount, truncated to whole cents.
func Percent(amount Cents, pct int) Cents {
	return amount * Cents(pct) / 100
}
