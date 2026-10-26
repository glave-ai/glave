/// Items and how many of each are in stock.
#[derive(Debug, Default)]
pub struct Stock {
    items: Vec<(String, u32)>,
}

impl Stock {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `count` of `name`, to any already in stock.
    pub fn add(&mut self, name: &str, count: u32) {
        self.items.push((name.to_owned(), count));
    }

    /// How many of `name` are in stock.
    pub fn count(&self, name: &str) -> u32 {
        self.items
            .iter()
            .find(|(item, _)| item == name)
            .map_or(0, |(_, count)| *count)
    }

    /// Takes `count` of `name` out of stock. When fewer are in stock, takes nothing and says
    /// so with `false`.
    pub fn take(&mut self, name: &str, count: u32) -> bool {
        match self.items.iter_mut().find(|(item, _)| item == name) {
            Some((_, have)) => {
                *have -= count;
                true
            }
            None => false,
        }
    }
}
