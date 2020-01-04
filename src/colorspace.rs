pub use crate::ffi::J_COLOR_SPACE as ColorSpace;

pub trait ColorSpaceExt {
    /// Number of channels (including unused alpha) in this color space
    fn num_components(&self) -> usize;
}

impl ColorSpaceExt for ColorSpace {
    fn num_components(&self) -> usize {
        match *self {
            ColorSpace::JCS_UNKNOWN => 0,
            ColorSpace::JCS_GRAYSCALE => 1,
            ColorSpace::JCS_RGB => 3,
            ColorSpace::JCS_YCbCr => 3,
            ColorSpace::JCS_CMYK => 4,
            ColorSpace::JCS_YCCK => 4,
            ColorSpace::JCS_EXT_RGB => 3,
            ColorSpace::JCS_EXT_RGBX => 4,
            ColorSpace::JCS_EXT_BGR => 3,
            ColorSpace::JCS_EXT_BGRX => 4,
            ColorSpace::JCS_EXT_XBGR => 4,
            ColorSpace::JCS_EXT_XRGB => 4,
            ColorSpace::JCS_EXT_RGBA => 4,
            ColorSpace::JCS_EXT_BGRA => 4,
            ColorSpace::JCS_EXT_ABGR => 4,
            ColorSpace::JCS_EXT_ARGB => 4,
            ColorSpace::JCS_RGB565 => 3,
        }
    }
}

#[test]
fn test() {
    use crate::ffi;
    assert_eq!(3, ffi::J_COLOR_SPACE::JCS_YCbCr.num_components());
    assert_eq!(3, ffi::J_COLOR_SPACE::JCS_RGB.num_components());
    assert_eq!(1, ffi::J_COLOR_SPACE::JCS_GRAYSCALE.num_components());
}
