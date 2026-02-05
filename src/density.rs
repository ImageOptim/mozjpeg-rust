#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PixelDensityUnit {
    /// No units
    PixelAspectRatio = 0,
    /// Pixels per inch
    Inches = 1,
    /// Pixels per centimeter
    Centimeters = 2,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PixelDensity {
    pub unit: PixelDensityUnit,
    pub x: u16,
    pub y: u16,
}

impl Default for PixelDensity {
    fn default() -> Self {
        Self {
            unit: PixelDensityUnit::PixelAspectRatio,
            x: 1,
            y: 1,
        }
    }
}
