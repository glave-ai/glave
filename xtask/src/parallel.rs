//! How many tests `cargo xtask ci` runs at once. On CI, as many as each runner chooses: the
//! machine is the run's alone. On a laptop, half its cores, so the tests leave room for the
//! person's other work and their own servers, and a busy machine does not turn slow tests
//! into failed ones. `GLAVE_TEST_THREADS` sets it either way.

/// The variable that sets how many tests run at once; the web tests read it too.
pub(crate) const VARIABLE: &str = "GLAVE_TEST_THREADS";

/// Whether this runs on CI, which sets `CI` (GitHub Actions does).
pub(crate) fn on_ci() -> bool {
    std::env::var_os("CI").is_some_and(|value| !value.is_empty())
}

/// How many tests run at once, and why; `None` leaves it to each runner.
pub(crate) fn tests() -> Result<Option<(usize, String)>, String> {
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    chosen(std::env::var(VARIABLE).ok().as_deref(), on_ci(), cores)
}

/// [`tests`], from the variable's value, whether this is CI, and the machine's cores.
fn chosen(
    variable: Option<&str>,
    ci: bool,
    cores: usize,
) -> Result<Option<(usize, String)>, String> {
    if let Some(value) = variable {
        return match value.trim().parse::<usize>() {
            Ok(threads) if threads > 0 => Ok(Some((threads, format!("{VARIABLE}={threads}")))),
            _ => Err(format!(
                "{VARIABLE} must be a number of tests to run at once, not `{value}`"
            )),
        };
    }
    if ci {
        return Ok(None);
    }
    let threads = (cores / 2).max(1);
    Ok(Some((
        threads,
        format!("half of this machine's {cores} cores; {VARIABLE} changes it"),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_laptop_runs_half_its_cores_and_ci_leaves_it_to_the_runners() {
        assert_eq!(chosen(None, false, 8).unwrap().unwrap().0, 4);
        assert_eq!(chosen(None, false, 1).unwrap().unwrap().0, 1);
        assert_eq!(chosen(None, true, 8).unwrap(), None);
    }

    #[test]
    fn the_variable_wins_and_must_be_a_count() {
        assert_eq!(chosen(Some("6"), false, 8).unwrap().unwrap().0, 6);
        assert_eq!(chosen(Some("2"), true, 8).unwrap().unwrap().0, 2);
        assert!(chosen(Some("0"), false, 8).is_err());
        assert!(chosen(Some("many"), false, 8).is_err());
    }
}
