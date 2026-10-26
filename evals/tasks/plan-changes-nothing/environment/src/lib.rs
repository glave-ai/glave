/// Degrees Celsius as degrees Fahrenheit.
pub fn to_fahrenheit(celsius: f64) -> f64 {
    celsius * 9.0 / 5.0 + 23.0
}

/// Degrees Fahrenheit as degrees Celsius.
pub fn to_celsius(fahrenheit: f64) -> f64 {
    (fahrenheit - 32.0) * 5.0 / 9.0
}
