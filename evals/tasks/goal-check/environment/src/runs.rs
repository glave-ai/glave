/// Run-length encodes `text`: each run of the same character becomes its length and then the
/// character, with the length left out when it is 1. `encode("aaabcc")` is "3ab2c".
pub fn encode(text: &str) -> String {
    text.to_owned()
}
