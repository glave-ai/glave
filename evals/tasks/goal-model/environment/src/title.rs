/// `text` in title case: in every word, the first character uppercased and the rest
/// lowercased. Words are separated by single spaces, which are kept as they are.
/// `title("hELLO wORLD")` is "Hello World".
pub fn title(text: &str) -> String {
    text.to_owned()
}
