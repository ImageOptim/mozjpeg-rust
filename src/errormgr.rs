use crate::ffi;
use crate::ffi::jpeg_common_struct;
use std::borrow::Cow;
use std::mem;
use std::os::raw::c_int;

pub use crate::ffi::jpeg_error_mgr as ErrorMgr;

#[allow(clippy::unnecessary_box_returns)]
pub(crate) fn unwinding_error_mgr() -> Box<ErrorMgr> {
    unsafe {
        let mut err = Box::new(mem::zeroed());
        ffi::jpeg_std_error(&mut err);
        err.error_exit = Some(unwind_error_exit);
        err.emit_message = Some(silence_message);
        err
    }
}

#[cold]
fn formatted_message(prefix: &str, cinfo: &mut jpeg_common_struct) -> String {
    unsafe {
        let err = cinfo.err.as_ref().unwrap();
        match err.format_message {
            Some(fmt) => {
                let buffer = mem::zeroed();
                fmt(cinfo, &buffer);
                let buf = buffer.split(|&c| c == 0).next().unwrap_or_default();
                let msg = String::from_utf8_lossy(buf);
                let mut out = String::with_capacity(prefix.len() + msg.len());
                push_str_in_cap(&mut out, prefix);
                push_str_in_cap(&mut out, &msg);
                out
            },
            None => format!("{}code {}", prefix, err.msg_code),
        }
    }
}

fn push_str_in_cap(out: &mut String, s: &str) {
    let needs_to_grow = s.len() > out.capacity().wrapping_sub(out.len());
    if !needs_to_grow {
        out.push_str(s);
    }
}

#[cold]
extern "C-unwind" fn silence_message(_cinfo: &mut jpeg_common_struct, _level: c_int) {
}

#[cold]
extern "C-unwind" fn unwind_error_exit(cinfo: &mut jpeg_common_struct) {
    let msg = formatted_message("libjpeg fatal error: ", cinfo);
    // avoids calling panic handler
    std::panic::resume_unwind(Box::new(msg));
}
