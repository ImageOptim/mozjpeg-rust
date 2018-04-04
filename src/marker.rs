extern crate libc;
extern crate mozjpeg_sys as ffi;

use self::libc::c_int;

/// Marker number identifier (APP0-APP14 and commment markers)
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
            Marker::COM
        } else {
            Marker::APP(num - self::ffi::jpeg_marker::APP0 as u8)
        }
    }
}

impl Into<c_int> for Marker {
    fn into(self) -> c_int {
        match self {
            Marker::APP(n) => n as c_int + self::ffi::jpeg_marker::APP0 as c_int,
            Marker::COM => self::ffi::jpeg_marker::COM as c_int,
        }
    }
}
