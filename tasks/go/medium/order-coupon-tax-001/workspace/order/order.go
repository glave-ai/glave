// Package order models a customer order and its line items.
package order

import "task/money"

// Line is a single ordered item.
type Line struct {
	Name  string
	Price money.Cents
	Qty   int
}

// Order is a collection of line items.
type Order struct {
	Lines []Line
}

// Subtotal is the sum of price*qty across all lines, before tax or discounts.
func (o Order) Subtotal() money.Cents {
	var sum money.Cents
	for _, l := range o.Lines {
		sum += l.Price * money.Cents(l.Qty)
	}
	return sum
}
