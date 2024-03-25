#[derive(Copy, Clone)]
pub enum PixelDensityUnit {
    /// No units
    PixelAspectRatio = 0,
    /// Pixels per inch
    Inches = 1,
    /// Pixels per centimeter
    Centimeters = 2,
}

pub struct PixelDensity {
    pub unit: PixelDensityUnit,
    pub x: u16,
    pub y: u16,
}

impl Default for PixelDensity {
    fn default() -> Self {
        PixelDensity {
            unit: PixelDensityUnit::PixelAspectRatio,
            x: 1,
            y: 1,
        }
    }
}
