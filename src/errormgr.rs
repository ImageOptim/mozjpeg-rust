use crate::ffi::jpeg_common_struct;
use crate::ffi;
use std::borrow::Cow;
use std::mem;
use std::os::raw::c_int;

pub use crate::ffi::jpeg_error_mgr as ErrorMgr;

pub fn panicking_error_mgr() -> ErrorMgr {
    unsafe {
        let mut err = mem::zeroed();
        ffi::jpeg_std_error(&mut err);
        err.error_exit = Some(panic_error_exit);
        err.emit_message = Some(silence_message);
        err
    }
}

fn formatted_message(cinfo: &mut jpeg_common_struct) -> String {
    unsafe {
        let err = cinfo.err.as_ref().unwrap();
        match err.format_message {
            Some(fmt) => {
                let buffer = mem::zeroed();
                fmt(cinfo, &buffer);
                String::from_utf8_lossy(&buffer[..]).into_owned()
            },
            None => format!("code {}", err.msg_code),
        }
    }
}

extern "C" fn silence_message(_cinfo: &mut jpeg_common_struct, _level: c_int) {
}

extern "C" fn panic_error_exit(cinfo: &mut jpeg_common_struct) {
    let mut msg = formatted_message(cinfo);
    msg.insert_str(0, "libjpeg fatal error: ");
    panic!(msg);
}
