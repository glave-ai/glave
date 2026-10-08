use super::Settings;

pub fn default_settings() -> Settings {
    Settings { warehouse: "main".into(), low_stock: 5 }
}

pub fn parse_settings(text: &str) -> Option<Settings> {
    let mut settings = default_settings();
    for line in text.lines() {
        let (key, value) = line.split_once('=')?;
        match key.trim() {
            "warehouse" => settings.warehouse = value.trim().to_owned(),
            "low_stock" => settings.low_stock = value.trim().parse().ok()?,
            _ => {}
        }
    }
    Some(settings)
}

pub fn read_settings_file(path: &std::path::Path) -> std::io::Result<String> {
    std::fs::read_to_string(path)
}
