#![feature(core)]
extern crate mozjpeg_sys as ffi;

pub use self::ffi::jpeg_error_mgr as ErrorMgr;
use self::ffi::jpeg_common_struct;
use ::std::mem;

#[repr(C)]
pub struct WrapperError {
    buf: *const u8,
    len: usize,
    cap: usize,
}

cpp!{{
    #include<stdexcept>

    struct WrapperError {
        char* buf;
        uintptr_t len;
        uintptr_t cap;
    };

    struct mozjpeg_rust_wrapper_exception: std::runtime_error {
        mozjpeg_rust_wrapper_exception(): std::runtime_error("") {}
        WrapperError error;
    };

    extern "C" WrapperError panic_error_exit_rust(void* cinfo);

    extern "C" void panic_error_exit(void* cinfo) {
        WrapperError error = panic_error_exit_rust(cinfo);
        mozjpeg_rust_wrapper_exception e;
        e.error = error;
        throw e;
    }
}}

#[no_mangle]
pub extern "C" fn panic_error_exit_rust(cinfo: &mut jpeg_common_struct) -> WrapperError {
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
        let s = format!("libjpeg fatal error: {}", msg);
        let e = WrapperError { buf: s.as_ptr(), len: s.len(), cap: s.capacity() };
        std::mem::forget(s);
        e
    }
}

pub trait PanicingErrorMgr {
    fn new() -> ErrorMgr {
        extern "C" {
            fn panic_error_exit(cinfo: &mut jpeg_common_struct);
        }
        unsafe{

            let mut err = mem::zeroed();
            ffi::jpeg_std_error(&mut err);
            err.error_exit = Some(panic_error_exit);
            err
        }
    }
}

impl PanicingErrorMgr for ErrorMgr {}
