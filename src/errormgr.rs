use crate::ffi::jpeg_common_struct;
use crate::ffi;
use std::borrow::Cow;
use std::mem;
use std::os::raw::c_int;

pub use crate::ffi::jpeg_error_mgr as ErrorMgr;

pub fn unwinding_error_mgr() -> ErrorMgr {
    unsafe {
        let mut err = mem::zeroed();
        ffi::jpeg_std_error(&mut err);
        err.error_exit = Some(unwind_error_exit);
        err.emit_message = Some(silence_message);
        err
    }
}

fn formatted_message(prefix: & str, cinfo: &mut jpeg_common_struct) -> String {
    unsafe {
        let err = cinfo.err.as_ref().unwrap();
        match err.format_message {
            Some(fmt) => {
                let buffer = mem::zeroed();
                fmt(cinfo, &buffer);
                let len = buffer.iter().take_while(|&&c| c != 0).count();
                format!("{}{}", prefix, String::from_utf8_lossy(&buffer[..len]))
            },
            None => format!("{}code {}", prefix, err.msg_code),
        }
    }
}

extern "C" fn silence_message(_cinfo: &mut jpeg_common_struct, _level: c_int) {
}

extern "C" fn unwind_error_exit(cinfo: &mut jpeg_common_struct) {
    let msg = formatted_message("libjpeg fatal error: ", cinfo);
    // avoids calling panic handler
    std::panic::resume_unwind(Box::new(msg));
}
