#![feature(core)]
extern crate mozjpeg_sys as ffi;

use self::ffi::jpeg_common_struct;
pub use self::ffi::jpeg_error_mgr as ErrorMgr;
use ::std::mem;

pub trait PanicingErrorMgr {
    fn new() -> ErrorMgr {
        unsafe {
            let mut err = mem::zeroed();
            ffi::jpeg_std_error(&mut err);
            err.error_exit = Some(<ErrorMgr as PanicingErrorMgr>::panic_error_exit);
            err
        }
    }

    extern "C" fn panic_error_exit(cinfo: &mut jpeg_common_struct) {
        unsafe {
            let err = cinfo.err.as_ref().unwrap();
            let msg: String = match err.format_message {
                Some(fmt) => {
                    let buffer = mem::zeroed();
                    fmt(cinfo, &buffer);
                    ::std::string::String::from_utf8_lossy(&buffer[..]).into_owned()
                },
                None => format!("code {}", err.msg_code),
            };
            panic!(format!("libjpeg fatal error: {}", msg));
        }
    }
}

impl PanicingErrorMgr for ErrorMgr {}
