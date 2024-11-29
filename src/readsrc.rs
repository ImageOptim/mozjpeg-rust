use crate::{fail, warn};
use mozjpeg_sys::boolean;
use mozjpeg_sys::jpeg_decompress_struct;
use mozjpeg_sys::JERR_BAD_LENGTH;
use mozjpeg_sys::{jpeg_common_struct, jpeg_resync_to_restart, jpeg_source_mgr};
use mozjpeg_sys::{JERR_FILE_READ, JERR_VIRTUAL_BUG};
use mozjpeg_sys::{JPOOL_IMAGE, JPOOL_PERMANENT, JWRN_JPEG_EOF};
use std::cell::UnsafeCell;
use std::io::{self, BufRead, BufReader, Read};
use std::marker::PhantomPinned;
use std::mem::MaybeUninit;
use std::os::raw::{c_int, c_long, c_uint, c_void};
use std::panic::{RefUnwindSafe, UnwindSafe};
use std::ptr;
use std::ptr::NonNull;

pub(crate) struct SourceMgr<R> {
    /// The `jpeg_source_mgr` has requirements that are tricky for Rust:
    /// * it must have a stable address,
    /// * it is mutated via `cinfo.src` raw pointer via C, while the `SourceMgr` stored elsewhere owns it.
    ///   This requires interior mutability and a non-exclusive ownership (`Box<UnsafeCell>` would be useless).
    inner_shared: *mut UnsafeCell<SourceMgrInner<R>>,
}

impl<R: UnwindSafe> UnwindSafe for SourceMgr<R> {}
impl<R: RefUnwindSafe> RefUnwindSafe for SourceMgr<R> {}

#[repr(C)]
pub(crate) struct SourceMgrInner<R> {
    pub(crate) iface: jpeg_source_mgr,
    to_consume: usize,
    reader: R,
    // jpeg_source_mgr callbacks get a pointer to the struct
    _pinned: PhantomPinned,
}

impl<R: BufRead> SourceMgr<R> {
    #[inline]
    pub(crate) fn new(reader: R) -> io::Result<Self> {
        let mut src = SourceMgrInner {
            iface: jpeg_source_mgr {
                next_input_byte: ptr::null_mut(),
                bytes_in_buffer: 0,
                init_source: Some(SourceMgrInner::<R>::init_source),
                fill_input_buffer: Some(SourceMgrInner::<R>::fill_input_buffer),
                skip_input_data: Some(SourceMgrInner::<R>::skip_input_data),
                resync_to_restart: Some(jpeg_resync_to_restart),
                term_source: Some(SourceMgrInner::<R>::term_source),
            },
            to_consume: 0,
            reader,
            _pinned: PhantomPinned,
        };
        src.fill_input_buffer_impl()?;
        Ok(Self {
            inner_shared: Box::into_raw(Box::new(UnsafeCell::new(src))),
        })
    }

    /// This will have the buffer in valid state only if libjpeg stopped decoding
    /// at an end of a marker, or `jpeg_consume_input` has been called.
    pub fn into_inner(mut self) -> R {
        unsafe {
            let mut inner = Box::from_raw(std::mem::replace(&mut self.inner_shared, ptr::null_mut()));
            inner.get_mut().return_unconsumed_data();

            #[cfg(debug_assertions)]
            inner.get_mut().poison_jpeg_source_mgr();

            inner.into_inner().reader
        }
    }

    /// Safety: `SourceMgr` can only be dropped after `cinfo.src` is set to NULL,
    /// or otherwise guaranteed not to be used any more via libjpeg.
    pub unsafe fn iface_c_ptr(&mut self) -> *mut jpeg_source_mgr {
        debug_assert!(!self.inner_shared.is_null());
        unsafe {
            ptr::addr_of_mut!((*UnsafeCell::raw_get(self.inner_shared)).iface)
        }
    }
}

impl<R> Drop for SourceMgr<R> {
    fn drop(&mut self) {
        if !self.inner_shared.is_null() {
            unsafe {
                #[cfg(not(debug_assertions))]
                let _ = Box::from_raw(self.inner_shared);

                #[cfg(debug_assertions)]
                Box::from_raw(self.inner_shared).get_mut().poison_jpeg_source_mgr();
            }
        }
    }
}

impl<R> SourceMgrInner<R> {
    /// Make any further use by libjpeg cause a crash
    #[cfg(debug_assertions)]
    unsafe fn poison_jpeg_source_mgr(&mut self) {
        extern "C-unwind" fn crash(_: &mut jpeg_decompress_struct) {
            panic!("cinfo.src dangling");
        }
        extern "C-unwind" fn crash_i(cinfo: &mut jpeg_decompress_struct) -> boolean {
            crash(cinfo); 0
        }
        extern "C-unwind" fn crash_s(cinfo: &mut jpeg_decompress_struct, _: c_long) {
            crash(cinfo);
        }
        extern "C-unwind" fn crash_r(cinfo: &mut jpeg_decompress_struct, _: c_int) -> boolean {
            crash(cinfo); 0
        }
        ptr::write_volatile(&mut self.iface, jpeg_source_mgr {
            next_input_byte: ptr::NonNull::dangling().as_ptr(),
            bytes_in_buffer: !0,
            init_source: Some(crash),
            fill_input_buffer: Some(crash_i),
            skip_input_data: Some(crash_s),
            resync_to_restart: Some(crash_r),
            term_source: Some(crash),
        });
    }
}

impl<R: BufRead> SourceMgrInner<R> {
    #[inline]
    unsafe fn cast(cinfo: &mut jpeg_decompress_struct) -> &mut Self {
        if let Some(maybe_aliased_src) = cinfo.src.cast::<UnsafeCell<Self>>().as_ref() {
            // UnsafeCell is intentionally accessed via shared reference. The libjpeg library is single-threaded,
            // so while there are other pointers to the cell, they're not used concurrently.
            let this = maybe_aliased_src.get();
            // Type alias to unify higher-ranked lifetimes
            type FnPtr<'a> = unsafe extern "C-unwind" fn(cinfo: &'a mut jpeg_decompress_struct);
            // This is a redundant safety check to ensure the struct is ours
            if Some::<FnPtr>(Self::init_source) == (*this).iface.init_source {
                return &mut *this;
            }
        }
        fail(&mut cinfo.common, JERR_VIRTUAL_BUG);
    }

    unsafe extern "C-unwind" fn init_source(cinfo: &mut jpeg_decompress_struct) {
        // Do nothing, buffer has been filled by new()
        let _s = Self::cast(cinfo);
        debug_assert!(!_s.iface.next_input_byte.is_null());
        debug_assert!(_s.iface.bytes_in_buffer > 0);
    }

    #[cold]
    fn set_buffer_to_eoi(&mut self) {
        debug_assert_eq!(self.to_consume, 0); // this should happen at eof

        // libjpeg doesn't treat it as error, but fakes it!
        self.iface.next_input_byte = [0xFF, 0xD9, 0xFF, 0xD9].as_ptr();
        self.iface.bytes_in_buffer = 4;
    }

    #[inline(never)]
    fn fill_input_buffer_impl(&mut self) -> io::Result<()> {
        // Do not call return_unconsumed_data() here.
        // here bytes_in_buffer may be != 0, because jdhuff.c doesn't update
        // the value after consuming the buffer.

        self.reader.consume(self.to_consume);
        self.to_consume = 0;

        let buf = self.reader.fill_buf()?;
        self.to_consume = buf.len();

        self.iface.next_input_byte = buf.as_ptr();
        self.iface.bytes_in_buffer = buf.len();

        if buf.is_empty() {
            // this is EOF
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        Ok(())
    }

    /// In typical applications, it should read fresh data
    ///    into the buffer (ignoring the current state of `next_input_byte` and
    ///    `bytes_in_buffer`)
    unsafe extern "C-unwind" fn fill_input_buffer(cinfo: &mut jpeg_decompress_struct) -> boolean {
        let this = Self::cast(cinfo);
        match this.fill_input_buffer_impl() {
            Ok(()) => 1,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                this.set_buffer_to_eoi();
                warn(&mut cinfo.common, JWRN_JPEG_EOF);
                // boolean returned by this function is for async I/O, not errors.
                1
            },
            Err(_) => {
                fail(&mut cinfo.common, JERR_FILE_READ);
            },
        }
    }

    /// libjpeg makes `bytes_in_buffer` up to date before calling this
    unsafe extern "C-unwind" fn skip_input_data(cinfo: &mut jpeg_decompress_struct, num_bytes: c_long) {
        if num_bytes <= 0 {
            return;
        }
        let this = Self::cast(cinfo);
        let mut num_bytes = usize::try_from(num_bytes).unwrap();

        loop {
            if this.iface.bytes_in_buffer > 0 {
                let skip_from_buffer = this.iface.bytes_in_buffer.min(num_bytes);
                this.iface.bytes_in_buffer -= skip_from_buffer;
                this.iface.next_input_byte = this.iface.next_input_byte.add(skip_from_buffer);
                num_bytes -= skip_from_buffer;
            }
            if num_bytes == 0 {
                break;
            }
            if this.fill_input_buffer_impl().is_err() {
                fail(&mut cinfo.common, JERR_FILE_READ);
            }
        }
    }

    fn return_unconsumed_data(&mut self) {
        let unconsumed = self.to_consume.saturating_sub(self.iface.bytes_in_buffer);
        self.to_consume = 0;
        self.reader.consume(unconsumed);
    }

    /// `jpeg_finish_decompress` consumes data up to EOI before calling this
    unsafe extern "C-unwind" fn term_source(cinfo: &mut jpeg_decompress_struct) {
        let this = Self::cast(cinfo);
        this.return_unconsumed_data();
    }
}
