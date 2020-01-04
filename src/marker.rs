use mozjpeg_sys as ffi;
use std::os::raw::c_int;

/// Marker number identifier (APP0-APP14 and comment markers)
///
/// For actual contents of markers, see `MarkerData`
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Marker {
    COM,
    APP(u8),
}

impl From<u8> for Marker {
    fn from(num: u8) -> Marker {
        if num == self::ffi::jpeg_marker::COM as u8 {
            Self::COM
        } else {
            Self::APP(num - self::ffi::jpeg_marker::APP0 as u8)
        }
    }
}

impl Into<c_int> for Marker {
    fn into(self) -> c_int {
        match self {
            Self::APP(n) => c_int::from(n) + self::ffi::jpeg_marker::APP0 as c_int,
            Self::COM => self::ffi::jpeg_marker::COM as c_int,
        }
    }
}
