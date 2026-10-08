mod settings;
mod stock;

fn main() {
    let settings = settings::load(std::path::Path::new("inventory.toml"));
    stock::report(&settings);
}
