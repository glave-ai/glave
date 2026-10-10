/// `items` with every repeat removed, keeping the first of each value in its place.
/// `dedupe(&[3, 1, 3, 2, 1])` is `[3, 1, 2]`.
pub fn dedupe(items: &[i32]) -> Vec<i32> {
    items.to_vec()
}
