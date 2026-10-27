orders.csv is this year's orders from our shop. An order's net value is its units times its
unit price, less its discount (0.1 is 10% off). Work these out and write them to answers.json,
as a JSON object with these keys:

- `q2_net_revenue`: the net value of the orders from April to June that weren't returned, to
  the cent.
- `top_region` and `top_region_median`: the region whose orders that weren't returned have the
  highest median net value, and that median, to the cent.
- `discount_units_corr`: the correlation (Pearson's) between discount and units, over every
  order, to 3 decimal places.
- `most_variable_category` and `its_stddev`: the category whose orders that weren't returned
  vary most in net value, by their sample standard deviation, and that deviation, to the cent.
