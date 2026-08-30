use crate::ffi;
use crate::ffi::jpeg_common_struct;
use std::borrow::Cow;
use std::cell::Cell;
use std::mem;
use std::os::raw::c_int;

pub use crate::ffi::jpeg_error_mgr as ErrorMgr;

/// Words in [`Warnings`]' bit set, one bit per message code.
///
/// `JMSG_LASTMSGCODE` is 130, so three words cover every code libjpeg defines. A code past
/// the end still counts, via [`Warnings::has_unnamed_code`].
const WARNING_WORDS: usize = 3;

/// How many codes [`Warnings`] can name individually.
const WARNING_CODES: c_int = (WARNING_WORDS * u64::BITS as usize) as c_int;

/// The recoverable conditions libjpeg reported.
///
/// A warning means libjpeg found the data damaged, substituted something, and carried on —
/// a bad Huffman code, a premature marker, a wrong restart marker, or a contradictory
/// progressive scan sequence all decode "successfully". Checking for an error does not
/// distinguish a repaired image from a sound one; this does.
///
/// Codes are the `JWRN_*` constants of `mozjpeg_sys::jerror`.
///
/// ```rust
/// # use mozjpeg::Decompress;
/// let decompress = Decompress::new_path("tests/test.jpg")?;
/// assert!(decompress.warnings().is_empty());
/// # Ok::<(), std::io::Error>(())
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Warnings {
    seen: [u64; WARNING_WORDS],
    unnamed: bool,
    total: u64,
}

impl Warnings {
    /// Whether nothing was reported. False for every damaged image, whatever the damage, so
    /// a caller need not enumerate codes to be safe.
    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.total == 0
    }

    /// Warnings emitted, counting repeats of one code separately.
    #[inline]
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.total
    }

    /// Whether `code` was reported.
    #[inline]
    #[must_use]
    pub const fn contains(&self, code: c_int) -> bool {
        match Self::position(code) {
            Some((word, bit)) => self.seen[word] & (1u64 << bit) != 0,
            None => false,
        }
    }

    /// Whether a warning arrived whose code this set cannot name.
    ///
    /// Only an application message table past `JMSG_LASTMSGCODE` can cause it, which this
    /// crate never installs. It keeps "no codes but not empty" explainable, so a caller
    /// deciding by code cannot mistake an unnameable warning for none.
    #[inline]
    #[must_use]
    pub const fn has_unnamed_code(&self) -> bool {
        self.unnamed
    }

    /// The distinct codes reported, ascending.
    pub fn codes(&self) -> impl Iterator<Item = c_int> + '_ {
        (0..WARNING_CODES).filter(move |&code| self.contains(code))
    }

    const fn position(code: c_int) -> Option<(usize, u32)> {
        if code < 0 || code >= WARNING_CODES {
            return None;
        }
        Some((code as usize / 64, code as u32 % 64))
    }
}

/// libjpeg's error manager plus the warning record.
///
/// `#[repr(C)]` with `base` first, so the `*mut ErrorMgr` libjpeg holds is also a
/// `*mut CountingErrorMgr` — libjpeg's own `my_error_mgr` idiom, which avoids taking
/// `client_data` from the application.
///
/// The counters are [`Cell`]s so neither side ever needs `&mut` to a struct libjpeg holds a
/// pointer into. That is what keeps its pointer valid to write through for the whole
/// decode, and it costs only `Sync`, which the surrounding `Decompress` lacks anyway.
#[repr(C)]
pub(crate) struct CountingErrorMgr {
    pub(crate) base: ErrorMgr,
    seen: [Cell<u64>; WARNING_WORDS],
    unnamed: Cell<bool>,
    total: Cell<u64>,
}

impl CountingErrorMgr {
    /// Wraps `base` without touching its callbacks, so a caller-supplied manager keeps its
    /// own `emit_message` and records nothing here.
    pub(crate) fn new(base: ErrorMgr) -> Self {
        Self {
            base,
            seen: [const { Cell::new(0) }; WARNING_WORDS],
            unnamed: Cell::new(false),
            total: Cell::new(0),
        }
    }

    pub(crate) fn warnings(&self) -> Warnings {
        let mut seen = [0; WARNING_WORDS];
        for (word, cell) in seen.iter_mut().zip(&self.seen) {
            *word = cell.get();
        }
        Warnings {
            seen,
            unnamed: self.unnamed.get(),
            total: self.total.get(),
        }
    }

    fn record(&self, code: c_int) {
        match Warnings::position(code) {
            Some((word, bit)) => self.seen[word].set(self.seen[word].get() | 1u64 << bit),
            None => self.unnamed.set(true),
        }
        self.total.set(self.total.get().saturating_add(1));
    }
}

#[allow(clippy::unnecessary_box_returns)]
pub(crate) fn unwinding_error_mgr() -> Box<CountingErrorMgr> {
    // SAFETY: `jpeg_std_error` only needs a writable `jpeg_error_mgr` to fill in. All-zero
    // is a valid value for one: every field is an integer, a C array, a raw pointer, or an
    // `Option<extern fn>` whose `None` is the null pattern.
    unsafe {
        let mut base: ErrorMgr = mem::zeroed();
        ffi::jpeg_std_error(&mut base);
        base.error_exit = Some(unwind_error_exit);
        base.emit_message = Some(record_message);
        Box::new(CountingErrorMgr::new(base))
    }
}

#[cold]
fn formatted_message(prefix: &str, cinfo: &mut jpeg_common_struct) -> String {
    unsafe {
        let err = cinfo.err.as_ref().unwrap();
        match err.format_message {
            Some(fmt) => {
                let mut buffer = mem::zeroed();
                let correct_fn_type = mem::transmute::<
                    unsafe extern "C-unwind" fn(cinfo: &mut jpeg_common_struct, buffer: &[u8; 80]),
                    unsafe extern "C-unwind" fn(cinfo: &mut jpeg_common_struct, buffer: &mut [u8; 80])>(fmt);
                (correct_fn_type)(cinfo, &mut buffer);
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

/// Records a recoverable condition instead of discarding it. Prints nothing, so a library
/// still does not write to the process's `stderr`.
#[cold]
extern "C-unwind" fn record_message(cinfo: &mut jpeg_common_struct, level: c_int) {
    // Negative is a warning; non-negative is a trace message, which stays silent.
    if level >= 0 {
        return;
    }

    let err = cinfo.err;
    if err.is_null() {
        return;
    }

    // SAFETY: `err` points at the `base` of a live `CountingErrorMgr`. This function is
    // reachable only as `base.emit_message`, installed only by `unwinding_error_mgr`, and
    // the only pointer given to libjpeg is `addr_of_mut!(mgr.base)`; `#[repr(C)]` puts
    // `base` at offset 0, so the cast recovers the outer struct. The manager is owned by
    // the `Decompress`/`Compress` libjpeg is running for, so it outlives the call. Only a
    // shared reference is taken and `record` mutates through `Cell`, so this cannot
    // conflict with libjpeg's own writes to `msg_code`.
    unsafe {
        let code = (*err).msg_code;
        (*err.cast_const().cast::<CountingErrorMgr>()).record(code);
    }
}

#[cold]
extern "C-unwind" fn unwind_error_exit(cinfo: &mut jpeg_common_struct) {
    let msg = formatted_message("libjpeg fatal error: ", cinfo);
    // avoids calling panic handler
    std::panic::resume_unwind(Box::new(msg));
}
