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
    fn from(num: u8) -> Self {
        if num == crate::ffi::jpeg_marker::COM as u8 {
            Self::COM
        } else {
            Self::APP(num - crate::ffi::jpeg_marker::APP0 as u8)
        }
    }
}

impl From<Marker> for c_int {
    fn from(val: Marker) -> Self {
        match val {
            Marker::APP(n) => c_int::from(n) + crate::ffi::jpeg_marker::APP0 as c_int,
            Marker::COM => crate::ffi::jpeg_marker::COM as c_int,
        }
    }
}
