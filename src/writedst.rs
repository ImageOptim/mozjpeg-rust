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
use std::cell::UnsafeCell;
use std::io;
use std::io::Write;
use std::marker::PhantomPinned;
use std::mem::MaybeUninit;
use std::os::raw::{c_int, c_long, c_uint};
use std::ptr;

pub(crate) struct DestinationMgr<W> {
    /// The `jpeg_destination_mgr` has requirements that are tricky for Rust:
    /// * it must have a stable address,
    /// * it is mutated via `cinfo.dest` raw pointer via C, while the `DestinationMgr` stored elsewhere owns it.
    ///   This requires interior mutability and a non-exclusive ownership (`Box<UnsafeCell>` would be useless).
    inner_shared: *mut UnsafeCell<DestinationMgrInner<W>>,
}

#[repr(C)]
struct DestinationMgrInner<W> {
    /// The `iface` is mutably aliased by C code. Must be the first field.
    iface: jpeg_destination_mgr,
    buf: Vec<u8>,
    writer: W,
    // jpeg_destination_mgr callbacks get a pointer to the struct
    _pinned: PhantomPinned,
}

impl<W: Write> DestinationMgr<W> {
    #[inline]
    pub fn new(writer: W, capacity: usize) -> Self {
        Self {
            inner_shared: Box::into_raw(Box::new(UnsafeCell::new(
                DestinationMgrInner {
                    iface: jpeg_destination_mgr {
                        next_output_byte: ptr::null_mut(),
                        free_in_buffer: 0,
                        init_destination: Some(DestinationMgrInner::<W>::init_destination),
                        empty_output_buffer: Some(DestinationMgrInner::<W>::empty_output_buffer),
                        term_destination: Some(DestinationMgrInner::<W>::term_destination),
                    },
                    // Can't use BufWriter, because it doesn't expose the unwritten buffer
                    buf: Vec::with_capacity(if capacity > 0 { capacity.min(i32::MAX as usize) } else { 4096 }),
                    writer,
                    _pinned: PhantomPinned,
                },
            ))),
        }
    }

    #[cfg(test)]
    fn iface(&mut self) -> &mut jpeg_destination_mgr {
        unsafe {
            &mut (*UnsafeCell::raw_get(self.inner_shared)).iface
        }
    }

    /// Must be called after `term_destination`
    pub fn into_inner(mut self) -> W {
        unsafe {
            #[cfg(debug_assertions)]
            self.poison_jpeg_destination_mgr();

            let inner = std::mem::replace(&mut self.inner_shared, ptr::null_mut());
            Box::from_raw(inner).into_inner().writer
        }
    }
}

impl<W> DestinationMgr<W> {
    /// Safety: `DestinationMgr` can only be dropped after `cinfo.dest` is set to NULL
    pub unsafe fn iface_c_ptr(&mut self) -> *mut jpeg_destination_mgr {
        debug_assert!(!self.inner_shared.is_null());
        unsafe {
            ptr::addr_of_mut!((*UnsafeCell::raw_get(self.inner_shared)).iface)
        }
    }

    /// Make any further use by libjpeg cause a crash
    #[cfg(debug_assertions)]
    unsafe fn poison_jpeg_destination_mgr(&mut self) {
        extern "C-unwind" fn crash(_: &mut jpeg_compress_struct) {
            panic!("cinfo.dest dangling");
        }
        extern "C-unwind" fn crash_i(cinfo: &mut jpeg_compress_struct) -> i32 {
            crash(cinfo); 0
        }
        ptr::write_volatile(self.iface_c_ptr(), jpeg_destination_mgr {
            next_output_byte: ptr::NonNull::dangling().as_ptr(),
            free_in_buffer: !0,
            init_destination: Some(crash),
            empty_output_buffer: Some(crash_i),
            term_destination: Some(crash)
        });
    }
}

impl<W> Drop for DestinationMgr<W> {
    fn drop(&mut self) {
        if !self.inner_shared.is_null() {
            unsafe {
                #[cfg(debug_assertions)]
                self.poison_jpeg_destination_mgr();

                let _ = Box::from_raw(self.inner_shared);
            }
        }
    }
}

impl<W: Write> DestinationMgrInner<W> {
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
        if let Some(maybe_aliased_dest) = cinfo.dest.cast::<UnsafeCell<Self>>().as_ref() {
            // UnsafeCell is intentionally accessed via shared reference. The libjpeg library is single-threaded,
            // so while there are other pointers to the cell, they're not used concurrently.
            let this = maybe_aliased_dest.get();
            // Type alias to unify higher-ranked lifetimes
            type FnPtr<'a> = unsafe extern "C-unwind" fn(cinfo: &'a mut jpeg_compress_struct);
            // This is a redundant safety check to ensure the struct is ours
            if Some::<FnPtr>(Self::init_destination) == (*this).iface.init_destination {
                return &mut *this;
            }
        }
        fail(&mut cinfo.common, JERR_BUFFER_SIZE);
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
        Self::cast(cinfo).reset_buffer();
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
                let init_destination = w.iface().init_destination.unwrap();
                let empty_output_buffer = w.iface().empty_output_buffer.unwrap();
                let term_destination = w.iface().term_destination.unwrap();

                let mut j: jpeg_compress_struct = std::mem::zeroed();
                j.dest = w.iface_c_ptr();
                (init_destination)(&mut j);
                assert!(w.iface().free_in_buffer > 0);
                if any_write_first {
                    while w.iface().free_in_buffer > 0 {
                        expected.push(123);
                        *w.iface().next_output_byte = 123;
                        w.iface().next_output_byte = w.iface().next_output_byte.add(1);
                        w.iface().free_in_buffer -= 1;
                    }
                    (empty_output_buffer)(&mut j);
                    assert!(w.iface().free_in_buffer > 0);
                    let slice = std::slice::from_raw_parts_mut(w.iface().next_output_byte, w.iface().free_in_buffer);
                    slice.iter_mut().enumerate().for_each(|(i, s)| *s = i as u8);
                    expected.extend_from_slice(slice);
                    w.iface().next_output_byte = w.iface().next_output_byte.add(1); // yes, can be invalid!
                    w.iface().free_in_buffer = 999; // yes, can be invalid!
                    (empty_output_buffer)(&mut j);
                    assert!(w.iface().free_in_buffer > 0);
                }
                let slice = std::slice::from_raw_parts_mut(w.iface().next_output_byte, w.iface().free_in_buffer-1);
                slice.iter_mut().enumerate().for_each(|(i, s)| *s = (i*17) as u8);
                expected.extend_from_slice(slice);
                w.iface().next_output_byte = w.iface().next_output_byte.add(slice.len());
                w.iface().free_in_buffer -= slice.len(); // now must be valid
                (term_destination)(&mut j);
                assert_eq!(expected, w.into_inner());
            }
        }
    }
}
