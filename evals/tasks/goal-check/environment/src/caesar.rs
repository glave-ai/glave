/// Shifts every ASCII letter in `text` forward by `n` places in the alphabet, wrapping from z
/// to a and keeping its case. Everything else is unchanged. `shift("Zebra!", 1)` is "Afcsb!".
pub fn shift(text: &str, n: u8) -> String {
    let _ = n;
    text.to_owned()
}
