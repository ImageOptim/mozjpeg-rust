use crate::boolean;
use crate::fail;
use crate::ffi::jpeg_compress_struct;
use crate::warn;
use mozjpeg_sys::jpeg_common_struct;
use mozjpeg_sys::jpeg_destination_mgr;
use mozjpeg_sys::JERR_BUFFER_SIZE;
use mozjpeg_sys::JERR_FILE_WRITE;
use mozjpeg_sys::JERR_INPUT_EOF;
use mozjpeg_sys::JWRN_JPEG_EOF;
use mozjpeg_sys::J_MESSAGE_CODE;
use mozjpeg_sys::{JPOOL_IMAGE, JPOOL_PERMANENT};
use std::io;
use std::io::Write;
use std::mem::MaybeUninit;
use std::os::raw::c_int;
use std::os::raw::c_long;
use std::os::raw::c_uint;
use std::ptr;
use std::ptr::NonNull;

#[repr(C)]
pub(crate) struct DestinationMgr<W> {
    pub(crate) iface: jpeg_destination_mgr,
    buf: Vec<u8>,
    writer: W,
}

impl<W: Write> DestinationMgr<W> {
    #[inline]
    pub fn new(writer: W, capacity: usize) -> Self {
        Self {
            iface: jpeg_destination_mgr {
                next_output_byte: ptr::null_mut(),
                free_in_buffer: 0,
                init_destination: Some(Self::init_destination),
                empty_output_buffer: Some(Self::empty_output_buffer),
                term_destination: Some(Self::term_destination),
            },
            // BufWriter doesn't expose unwritten buffer
            buf: Vec::with_capacity(if capacity > 0 { capacity.min(i32::MAX as usize) } else { 4096 }),
            writer,
        }
    }

    /// Must be called after `term_destination`
    pub fn into_inner(self) -> W {
        self.writer
    }

    fn reset_buffer(&mut self) {
        self.buf.clear();
        let spare_capacity = self.buf.spare_capacity_mut();
        self.iface.next_output_byte = spare_capacity.as_mut_ptr().cast();
        self.iface.free_in_buffer = spare_capacity.len();
    }

    unsafe fn write_buffer(&mut self, full: bool) -> Result<(), J_MESSAGE_CODE> {
        let buf = self.buf.spare_capacity_mut();
        let used_capacity = if full { buf.len() } else {
            buf.len().checked_sub(self.iface.free_in_buffer).ok_or(JERR_BUFFER_SIZE)?
        };
        if used_capacity > 0 {
            unsafe {
                self.buf.set_len(used_capacity);
            }
            self.writer.write_all(&self.buf).map_err(|_| JERR_FILE_WRITE)?;
        }
        self.reset_buffer();
        Ok(())
    }

    unsafe fn cast(cinfo: &mut jpeg_compress_struct) -> &mut Self {
        let this: &mut Self = &mut *cinfo.dest.cast();
        // Type alias to unify higher-ranked lifetimes
        type FnPtr<'a> = unsafe extern "C-unwind" fn(cinfo: &'a mut jpeg_compress_struct);
        // This is a redundant safety check to ensure the struct is ours
        if Some::<FnPtr>(Self::init_destination) != this.iface.init_destination {
            fail(&mut cinfo.common, JERR_BUFFER_SIZE);
        }
        this
    }

    /// This is called by `jcphuff`'s `dump_buffer()`, which does NOT keep
    /// the position up to date, and expects full buffer write every time.
    unsafe extern "C-unwind" fn empty_output_buffer(cinfo: &mut jpeg_compress_struct) -> boolean {
        let this = Self::cast(cinfo);
        if let Err(code) = this.write_buffer(true) {
            fail(&mut cinfo.common, code);
        }
        1
    }

    unsafe extern "C-unwind" fn init_destination(cinfo: &mut jpeg_compress_struct) {
        let this = Self::cast(cinfo);
        this.reset_buffer();
    }

    unsafe extern "C-unwind" fn term_destination(cinfo: &mut jpeg_compress_struct) {
        let this = Self::cast(cinfo);
        if let Err(code) = this.write_buffer(false) {
            fail(&mut cinfo.common, code);
        }
        if this.writer.flush().is_err() {
            fail(&mut cinfo.common, JERR_FILE_WRITE);
        }
        this.iface.free_in_buffer = 0;
    }
}

#[test]
fn w() {
    for any_write_first in [true, false] {
    for capacity in [0,1,2,3,5,10,255,256,4096] {
        let mut w = DestinationMgr::new(Vec::new(), capacity);
        let mut expected = Vec::new();
        unsafe {
            let mut j: jpeg_compress_struct = std::mem::zeroed();
            j.dest = &mut w.iface;
            (w.iface.init_destination.unwrap())(&mut j);
            assert!(w.iface.free_in_buffer > 0);
            if any_write_first {
                while w.iface.free_in_buffer > 0 {
                    expected.push(123);
                    *w.iface.next_output_byte = 123;
                    w.iface.next_output_byte = w.iface.next_output_byte.add(1);
                    w.iface.free_in_buffer -= 1;
                }
                (w.iface.empty_output_buffer.unwrap())(&mut j);
                assert!(w.iface.free_in_buffer > 0);
                let slice = std::slice::from_raw_parts_mut(w.iface.next_output_byte, w.iface.free_in_buffer);
                slice.iter_mut().enumerate().for_each(|(i, s)| *s = i as u8);
                expected.extend_from_slice(slice);
                w.iface.next_output_byte = w.iface.next_output_byte.add(1); // yes, can be invalid!
                w.iface.free_in_buffer = 999; // yes, can be invalid!
                (w.iface.empty_output_buffer.unwrap())(&mut j);
                assert!(w.iface.free_in_buffer > 0);
            }
            let slice = std::slice::from_raw_parts_mut(w.iface.next_output_byte, w.iface.free_in_buffer-1);
            slice.iter_mut().enumerate().for_each(|(i, s)| *s = (i*17) as u8);
            expected.extend_from_slice(slice);
            w.iface.next_output_byte = w.iface.next_output_byte.add(slice.len());
            w.iface.free_in_buffer -= slice.len(); // now must be valid
            (w.iface.term_destination.unwrap())(&mut j);
            assert_eq!(expected, w.into_inner());
        }
    }}
}
