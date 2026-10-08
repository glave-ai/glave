pub mod loader;

pub struct Settings {
    pub warehouse: String,
    pub low_stock: u32,
}

pub fn load(path: &std::path::Path) -> Settings {
    let text = loader::read_settings_file(path).unwrap_or_default();
    loader::parse_settings(&text).unwrap_or_else(loader::default_settings)
}
