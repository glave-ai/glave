//! Statistics SQLite lacks, as Postgres names them: `stddev` and `variance` of a sample,
//! `stddev_pop` and `var_pop` of a whole population, and `corr`, Pearson's correlation of two
//! columns. Empty values are left out, as `avg` leaves them out; each is computed in one pass
//! (Welford's), so large numbers keep their precision.

use rusqlite::functions::{Aggregate, Context, FunctionFlags};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, Error};

/// Adds the functions to `connection`.
pub(crate) fn add(connection: &Connection) -> rusqlite::Result<()> {
    let flags = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
    for (name, spread) in [
        ("stddev", Spread::SampleDeviation),
        ("stddev_samp", Spread::SampleDeviation),
        ("stddev_pop", Spread::PopulationDeviation),
        ("variance", Spread::SampleVariance),
        ("var_samp", Spread::SampleVariance),
        ("var_pop", Spread::PopulationVariance),
    ] {
        connection.create_aggregate_function(name, 1, flags, spread)?;
    }
    connection.create_aggregate_function("corr", 2, flags, Correlation)
}

/// How far a column's values spread.
#[derive(Clone, Copy)]
enum Spread {
    SampleDeviation,
    PopulationDeviation,
    SampleVariance,
    PopulationVariance,
}

/// What one pass over a column keeps: how many values, their mean, and the sum of their
/// squared distances from it.
#[derive(Default)]
struct Moments {
    count: f64,
    mean: f64,
    squares: f64,
}

impl Moments {
    fn add(&mut self, value: f64) -> f64 {
        self.count += 1.0;
        let before = value - self.mean;
        self.mean += before / self.count;
        self.squares += before * (value - self.mean);
        before
    }
}

impl Aggregate<Moments, Option<f64>> for Spread {
    fn init(&self, _: &mut Context<'_>) -> rusqlite::Result<Moments> {
        Ok(Moments::default())
    }

    fn step(&self, context: &mut Context<'_>, moments: &mut Moments) -> rusqlite::Result<()> {
        if let Some(value) = number(context, 0, self.name())? {
            moments.add(value);
        }
        Ok(())
    }

    fn finalize(
        &self,
        _: &mut Context<'_>,
        moments: Option<Moments>,
    ) -> rusqlite::Result<Option<f64>> {
        let Some(Moments { count, squares, .. }) = moments else {
            return Ok(None);
        };
        let (sample, deviation) = match self {
            Self::SampleDeviation => (true, true),
            Self::PopulationDeviation => (false, true),
            Self::SampleVariance => (true, false),
            Self::PopulationVariance => (false, false),
        };
        let over = if sample { count - 1.0 } else { count };
        if over < 1.0 {
            return Ok(None);
        }
        let variance = squares / over;
        Ok(Some(if deviation { variance.sqrt() } else { variance }))
    }
}

impl Spread {
    fn name(self) -> &'static str {
        match self {
            Self::SampleDeviation | Self::PopulationDeviation => "stddev",
            Self::SampleVariance | Self::PopulationVariance => "variance",
        }
    }
}

/// Pearson's correlation of two columns.
struct Correlation;

/// What one pass over two columns keeps: each one's moments, and how they move together.
#[derive(Default)]
struct Together {
    x: Moments,
    y: Moments,
    products: f64,
}

impl Aggregate<Together, Option<f64>> for Correlation {
    fn init(&self, _: &mut Context<'_>) -> rusqlite::Result<Together> {
        Ok(Together::default())
    }

    fn step(&self, context: &mut Context<'_>, together: &mut Together) -> rusqlite::Result<()> {
        // A row counts only when it has both values, as in Postgres.
        if let (Some(x), Some(y)) = (number(context, 0, "corr")?, number(context, 1, "corr")?) {
            let before_x = together.x.add(x);
            together.y.add(y);
            together.products += before_x * (y - together.y.mean);
        }
        Ok(())
    }

    fn finalize(
        &self,
        _: &mut Context<'_>,
        together: Option<Together>,
    ) -> rusqlite::Result<Option<f64>> {
        Ok(together.and_then(|Together { x, y, products }| {
            let spread = (x.squares * y.squares).sqrt();
            (x.count >= 2.0 && spread > 0.0).then(|| products / spread)
        }))
    }
}

/// Argument `at` as a number; `None` when it is empty. Text that reads as a number counts as
/// that number; other text is refused, saying which function it was given to.
#[expect(
    clippy::cast_precision_loss,
    reason = "a whole number as a float, as avg takes it"
)]
fn number(context: &Context<'_>, at: usize, function: &str) -> rusqlite::Result<Option<f64>> {
    let refused = |what: String| {
        Error::UserFunctionError(format!("{function} takes numbers, and was given {what}").into())
    };
    match context.get_raw(at) {
        ValueRef::Null => Ok(None),
        ValueRef::Integer(whole) => Ok(Some(whole as f64)),
        ValueRef::Real(number) => Ok(Some(number)),
        ValueRef::Text(text) => {
            let text = String::from_utf8_lossy(text);
            match text.trim().parse::<f64>() {
                Ok(number) if number.is_finite() => Ok(Some(number)),
                _ => Err(refused(format!("the text {text:?}"))),
            }
        }
        ValueRef::Blob(_) => Err(refused("bytes".to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(sql: &str) -> Option<f64> {
        let connection = Connection::open_in_memory().unwrap();
        add(&connection).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE t (x, y); INSERT INTO t VALUES \
                 (2, 1), (4, 3), (4, 2), (4, 5), (5, 4), (5, 6), (7, 7), (9, 8), (NULL, 3);",
            )
            .unwrap();
        connection.query_row(sql, [], |row| row.get(0)).unwrap()
    }

    fn near(got: Option<f64>, want: f64) {
        let got = got.unwrap();
        assert!((got - want).abs() < 1e-12, "{got} is not {want}");
    }

    #[test]
    fn spreads_are_a_sample_s_or_a_population_s() {
        near(one("SELECT stddev_pop(x) FROM t"), 2.0);
        near(one("SELECT var_pop(x) FROM t"), 4.0);
        near(one("SELECT variance(x) FROM t"), 32.0 / 7.0);
        near(one("SELECT stddev(x) FROM t"), (32.0_f64 / 7.0).sqrt());
        near(one("SELECT stddev_samp(x) FROM t"), (32.0_f64 / 7.0).sqrt());
        near(one("SELECT var_samp(x) FROM t"), 32.0 / 7.0);
        assert_eq!(one("SELECT stddev(x) FROM t WHERE x = 2"), None);
        near(one("SELECT stddev_pop(x) FROM t WHERE x = 2"), 0.0);
        assert_eq!(one("SELECT stddev(x) FROM t WHERE x > 100"), None);
    }

    #[test]
    fn large_numbers_keep_their_precision() {
        // Summing squares first would lose every digit here: 1e9 squared is past what a float
        // holds to the unit.
        let got = one("SELECT stddev_pop(x + 1e9) FROM t").unwrap();
        assert!((got - 2.0).abs() < 1e-6, "{got} is not 2");
    }

    #[test]
    fn correlation_is_pearson_s_over_rows_with_both_values() {
        // Worked out by hand: the co-moment is 33 over sqrt(32 * 42).
        near(
            one("SELECT corr(x, y) FROM t"),
            33.0 / (32.0_f64 * 42.0).sqrt(),
        );
        near(one("SELECT corr(x, x * 2 + 1) FROM t"), 1.0);
        near(one("SELECT corr(x, -x) FROM t"), -1.0);
        assert_eq!(one("SELECT corr(x, 3) FROM t"), None);
    }

    #[test]
    fn text_that_is_no_number_is_refused_by_name() {
        let connection = Connection::open_in_memory().unwrap();
        add(&connection).unwrap();
        near(
            connection
                .query_row(
                    "SELECT stddev_pop(v) FROM (SELECT '1' AS v UNION ALL SELECT 3)",
                    [],
                    |row| row.get(0),
                )
                .unwrap(),
            1.0,
        );
        let error = connection
            .query_row("SELECT stddev('North')", [], |row| {
                row.get::<_, Option<f64>>(0)
            })
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("stddev takes numbers, and was given the text \"North\""),
            "{error}"
        );
    }
}
