use crate::settings::Settings;

pub fn report(settings: &Settings) {
    println!("{}: alert below {}", settings.warehouse, settings.low_stock);
}
