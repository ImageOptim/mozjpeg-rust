use crate::{fail, warn};
use mozjpeg_sys::boolean;
use mozjpeg_sys::jpeg_decompress_struct;
use mozjpeg_sys::{JERR_INPUT_EOF, JERR_BUFFER_SIZE, JERR_FILE_READ};
use mozjpeg_sys::{jpeg_common_struct, jpeg_resync_to_restart, jpeg_source_mgr};
use mozjpeg_sys::{JWRN_JPEG_EOF, JPOOL_IMAGE, JPOOL_PERMANENT};
use std::io::{Read, BufRead, BufReader};
use std::mem::MaybeUninit;
use std::os::raw::{c_int, c_long, c_uint};
use std::ptr::NonNull;
use std::ptr;

#[repr(C)]
pub(crate) struct SourceMgr<R> {
    iface: jpeg_source_mgr,
    reader: R,
}

impl<R: BufRead> SourceMgr<R> {
    pub(crate) fn set_src(cinfo: &mut jpeg_decompress_struct, reader: R) -> Result<(), ()> {
        unsafe {
            if !cinfo.src.is_null() {
                return Err(());
            }

            let alloc_small = (*cinfo.common.mem).alloc_small.ok_or(())?;
            // JPOOL_IMAGE should be enough, but I'm not going to risk UAF for a few bytes. TJ API uses permanent.
            let head: *mut MaybeUninit<Self> = (alloc_small)(&mut cinfo.common, JPOOL_PERMANENT, std::mem::size_of::<Self>()).cast();
            let mut head = NonNull::new(head).ok_or(())?;
            head.as_mut().write(Self::new(reader));
            cinfo.src = &mut head.as_mut().assume_init_mut().iface;
        }
        Ok(())
    }

    #[inline]
    fn new(reader: R) -> Self {
        Self {
            iface: jpeg_source_mgr {
                next_input_byte: ptr::null_mut(),
                bytes_in_buffer: 0,
                init_source: Some(Self::init_source),
                fill_input_buffer: Some(Self::fill_input_buffer),
                skip_input_data: Some(Self::skip_input_data),
                resync_to_restart: Some(jpeg_resync_to_restart),
                term_source: Some(Self::term_source),
            },
            reader,
        }
    }

    unsafe fn cast(cinfo: &mut jpeg_decompress_struct) -> &mut Self {
        let this: &mut Self = &mut *cinfo.src.cast();
        // Type alias to unify higher-ranked lifetimes
        type FnPtr<'a> = unsafe extern "C" fn(cinfo: &'a mut jpeg_decompress_struct);
        // This is a redundant safety check to ensure the struct is ours
        if Some::<FnPtr>(Self::init_source) != this.iface.init_source {
            fail(&mut cinfo.common, JERR_BUFFER_SIZE);
        }
        this
    }

    unsafe extern "C" fn init_source(cinfo: &mut jpeg_decompress_struct) {
        let _ = Self::cast(cinfo);
    }

    fn set_buffer_to_eoi(&mut self) {
        // libjpeg doesn't treat it as error, but fakes it!
        self.iface.next_input_byte = [0xFF, 0xD9, 0xFF, 0xD9].as_ptr();
        self.iface.bytes_in_buffer = 4;
    }

    fn fill_input_buffer_impl(&mut self) -> Result<(), c_int> {
        // here bytes_in_buffer may be != 0, because jdhuff.c doesn't update
        // the value after consuming the buffer.

        let buf = self.reader.fill_buf().map_err(|_| JERR_FILE_READ)?;
        if buf.is_empty() {
            // this is EOF
            return Err(JERR_INPUT_EOF);
        }

        self.iface.next_input_byte = buf.as_ptr();
        self.iface.bytes_in_buffer = buf.len() as _;
        self.reader.consume(self.iface.bytes_in_buffer);
        Ok(())
    }

    unsafe extern "C" fn fill_input_buffer(cinfo: &mut jpeg_decompress_struct) -> boolean {
        let this = Self::cast(cinfo);
        match this.fill_input_buffer_impl() {
            Ok(()) => 1,
            Err(JERR_INPUT_EOF) => {
                this.set_buffer_to_eoi();
                warn(&mut cinfo.common, JWRN_JPEG_EOF);
                // boolean returned by this function is for async I/O, not errors.
                1
            },
            Err(e) => {
                fail(&mut cinfo.common, e);
            }
        }
    }

    unsafe extern "C" fn skip_input_data(cinfo: &mut jpeg_decompress_struct, num_bytes: c_long) {
        let this = Self::cast(cinfo);
        let mut num_bytes = num_bytes as usize;

        loop {
            if this.iface.bytes_in_buffer > 0 {
                let skip_from_buffer = this.iface.bytes_in_buffer.min(num_bytes);
                this.iface.bytes_in_buffer -= skip_from_buffer;
                num_bytes -= skip_from_buffer;
            }
            if num_bytes == 0 {
                break;
            }
            if let Err(code) = this.fill_input_buffer_impl() {
                fail(&mut cinfo.common, code);
            }
        }
    }

    unsafe extern "C" fn term_source(cinfo: &mut jpeg_decompress_struct) {
        let this = Self::cast(cinfo);
        let _: Self = ptr::read(this); // runs destructor for reader
        cinfo.src = ptr::null_mut(); // this is in jpeg's mempool
    }
}
