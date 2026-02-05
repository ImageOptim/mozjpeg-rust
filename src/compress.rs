use crate::colorspace::ColorSpaceExt;
use crate::component::CompInfo;
use crate::component::CompInfoExt;
use crate::errormgr::unwinding_error_mgr;
use crate::errormgr::ErrorMgr;
use crate::fail;
use crate::ffi;
use crate::ffi::boolean;
use crate::ffi::jpeg_compress_struct;
use crate::ffi::DCTSIZE;
use crate::ffi::JDIMENSION;
use crate::ffi::JPEG_LIB_VERSION;
use crate::ffi::J_BOOLEAN_PARAM;
use crate::ffi::J_INT_PARAM;
use crate::marker::Marker;
use crate::qtable::QTable;
use crate::writedst::DestinationMgr;
use crate::{colorspace::ColorSpace, PixelDensity};
use arrayvec::ArrayVec;
use std::cmp::min;
use std::io;
use std::marker::PhantomPinned;
use std::mem;
use std::os::raw::{c_int, c_uchar, c_uint, c_ulong, c_void};
use std::ptr;
use std::ptr::addr_of_mut;
use std::slice;

/// Max sampling factor is 2
pub const MAX_MCU_HEIGHT: usize = 16;
/// Codec doesn't allow more channels than this
pub const MAX_COMPONENTS: usize = 4;

/// Pending configuration that will be applied at `start_compress()` time.
///
/// Settings are collected here so they can be applied in the correct order,
/// making the order of setter calls irrelevant to the user.
///
/// # Application Order
///
/// Settings are applied in this order at `start_compress()`:
/// 1. Image dimensions (width, height)
/// 2. Output colorspace (if changed)
/// 3. Int params (`scan_mode`, `fastest_defaults`)
/// 4. Call `jpeg_set_defaults()` to set up internal state
/// 5. All other settings (quality, smoothing, progressive, etc.)
/// 6. User callbacks for raw cinfo access (applied last)
///
/// This ensures settings aren't accidentally reset by `jpeg_set_defaults()`.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct PendingConfig {
    // === Image basics - applied FIRST ===
    pub(crate) width: Option<u32>,
    pub(crate) height: Option<u32>,
    pub(crate) output_colorspace: Option<ColorSpace>,

    // === Settings that control jpeg_set_defaults() behavior ===
    pub(crate) scan_mode: Option<ScanMode>,
    pub(crate) fastest_defaults: bool,

    // === All other settings - applied AFTER jpeg_set_defaults() ===
    pub(crate) quality: Option<i32>,
    pub(crate) luma_qtable: Option<QTable>,
    pub(crate) chroma_qtable: Option<QTable>,
    /// When true, quantization table values are clamped to 1-255 (8-bit DQT).
    /// When false (default), values can go up to 32767 (16-bit DQT).
    pub(crate) force_8bit_quantization: bool,
    pub(crate) smoothing_factor: Option<u8>,
    pub(crate) pixel_density: Option<PixelDensity>,
    pub(crate) optimize_coding: Option<bool>,
    pub(crate) optimize_scans: Option<bool>,
    pub(crate) use_scans_in_trellis: Option<bool>,
    pub(crate) progressive_mode: bool,
    pub(crate) raw_data_in: Option<bool>,
    pub(crate) subsampling: Option<Vec<(i32, i32)>>,
}

/// Create a new JPEG file from pixels
///
/// Wrapper for `jpeg_compress_struct`
///
/// # Configuration Order Independence
///
/// As of version 0.11.0, settings can be applied in any order. They are
/// collected and applied at [`start_compress()`](Self::start_compress) time
/// in the correct order.
///
/// ```
/// use mozjpeg::{Compress, ColorSpace, ScanMode};
///
/// let mut comp = Compress::new(ColorSpace::JCS_RGB);
/// comp.set_size(64, 64);
///
/// // These can be called in any order - same result either way
/// comp.set_smoothing_factor(50);
/// comp.set_scan_optimization_mode(ScanMode::Auto);
/// comp.set_quality(85.0);
/// ```
///
/// # Raw Access
///
/// For advanced use cases requiring direct `cinfo` access, use
/// [`mutate_cinfo_last()`](Self::mutate_cinfo_last) which runs a callback
/// after all configuration is applied:
///
/// ```
/// use mozjpeg::{Compress, ColorSpace};
///
/// let mut comp = Compress::new(ColorSpace::JCS_RGB);
/// comp.set_size(64, 64);
/// comp.set_quality(85.0);
///
/// // This callback runs at start_compress() time, after all config is applied
/// comp.mutate_cinfo_last(|cinfo| {
///     // Direct cinfo access here
///     // e.g., cinfo.smoothing_factor = 50;
/// });
/// ```
pub struct Compress {
    cinfo: jpeg_compress_struct,

    /// It's `Box<ErrorMgr>`, but `cinfo` references `own_err`,
    /// so I need talismans to ward off nasal demons haunting self-referential structs
    own_err: *mut ErrorMgr,
    _it_is_self_referential: PhantomPinned,

    /// Pending configuration to be applied at start_compress() time.
    pending: PendingConfig,

    /// Snapshot of configuration that was already applied.
    /// Used for incremental application when deprecated methods trigger early apply.
    applied_snapshot: Option<PendingConfig>,

    /// Callbacks for raw cinfo access, run LAST after all other config.
    /// Stored separately from PendingConfig because closures can't be Clone/PartialEq.
    #[allow(clippy::type_complexity)]
    raw_callbacks: Vec<Box<dyn FnOnce(&mut jpeg_compress_struct)>>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ScanMode {
    AllComponentsTogether = 0,
    /// Can flash grayscale or green-tinted images
    ScanPerComponent = 1,
    Auto = 2,
}

pub struct CompressStarted<W> {
    compress: Compress,
    /// Safety: sensitive to drop order. Needs to be dropped after `Compress`
    dest_mgr: DestinationMgr<W>,
}

impl Compress {
    /// Compress image using input in this colorspace.
    ///
    /// ## Panics
    ///
    /// You need to wrap all use of this library in `std::panic::catch_unwind()`
    ///
    /// By default errors cause unwind (panic) and unwind through the C code,
    /// which strictly speaking is not guaranteed to work in Rust (but seems to work fine, at least on x86-64 and ARM).
    #[must_use]
    pub fn new(color_space: ColorSpace) -> Self {
        Self::new_err(unwinding_error_mgr(), color_space)
    }

    /// Use a specific error handler instead of the default unwinding one.
    ///
    /// Note that the error handler must either abort the process or unwind,
    /// it can't gracefully return due to the design of libjpeg.
    ///
    /// `color_space` refers to input color space
    #[must_use]
    pub fn new_err(err: Box<ErrorMgr>, color_space: ColorSpace) -> Self {
        unsafe {
            let mut newself = Self {
                cinfo: mem::zeroed(),
                own_err: Box::into_raw(err),
                _it_is_self_referential: PhantomPinned,
                pending: PendingConfig::default(),
                applied_snapshot: None,
                raw_callbacks: Vec::new(),
            };
            newself.cinfo.common.err = addr_of_mut!(*newself.own_err);

            let s = mem::size_of_val(&newself.cinfo);
            ffi::jpeg_CreateCompress(&mut newself.cinfo, JPEG_LIB_VERSION, s);

            newself.cinfo.in_color_space = color_space;
            newself.cinfo.input_components = color_space.num_components() as c_int;
            ffi::jpeg_set_defaults(&mut newself.cinfo);

            newself
        }
    }

    #[doc(hidden)]
    #[deprecated(note = "Give a Vec to start_compress instead")]
    pub fn set_mem_dest(&self) {}

    /// Apply all pending configuration incrementally.
    ///
    /// Called automatically by `start_compress()` and by deprecated methods like
    /// `components_mut()`. Uses a snapshot to track what was previously applied,
    /// so only new/changed settings are applied on subsequent calls.
    ///
    /// Settings are applied in the correct order to ensure `jpeg_set_defaults()`
    /// doesn't reset user settings.
    fn apply_pending_config(&mut self) {
        let default_snap = PendingConfig::default();
        let snap = self.applied_snapshot.as_ref().unwrap_or(&default_snap);

        // === Phase 1: Image dimensions (only if changed) ===
        if self.pending.width != snap.width {
            if let Some(width) = self.pending.width {
                self.cinfo.image_width = width as JDIMENSION;
            }
        }
        if self.pending.height != snap.height {
            if let Some(height) = self.pending.height {
                self.cinfo.image_height = height as JDIMENSION;
            }
        }

        // === Phase 2: Output colorspace (only if changed) ===
        if self.pending.output_colorspace != snap.output_colorspace {
            if let Some(colorspace) = self.pending.output_colorspace {
                unsafe {
                    ffi::jpeg_set_colorspace(&mut self.cinfo, colorspace);
                }
            }
        }

        // === Phase 3: Set int params BEFORE jpeg_set_defaults() ===
        let scan_mode_changed = self.pending.scan_mode != snap.scan_mode;
        let fastest_changed = self.pending.fastest_defaults != snap.fastest_defaults;

        if scan_mode_changed {
            if let Some(mode) = self.pending.scan_mode {
                unsafe {
                    ffi::jpeg_c_set_int_param(
                        &mut self.cinfo,
                        J_INT_PARAM::JINT_DC_SCAN_OPT_MODE,
                        mode as c_int,
                    );
                }
            }
        }

        if fastest_changed && self.pending.fastest_defaults {
            unsafe {
                ffi::jpeg_c_set_int_param(
                    &mut self.cinfo,
                    J_INT_PARAM::JINT_COMPRESS_PROFILE,
                    ffi::JINT_COMPRESS_PROFILE_VALUE::JCP_FASTEST as c_int,
                );
            }
        }

        // === Phase 4: Call jpeg_set_defaults() if int params changed ===
        // jpeg_set_defaults() resets many settings, so we track if it was called
        // to force reapplication of affected settings
        let defaults_called = (scan_mode_changed || fastest_changed)
            && (self.pending.scan_mode.is_some() || self.pending.fastest_defaults);
        if defaults_called {
            unsafe {
                ffi::jpeg_set_defaults(&mut self.cinfo);
            }
        }

        // === Phase 5: Apply all other settings ===
        // If jpeg_set_defaults() was called, reapply ALL settings regardless of snapshot
        // because it resets quality, smoothing, progressive mode, etc.

        // Quality / quantization tables
        let force_8bit = self.pending.force_8bit_quantization;
        if defaults_called || self.pending.quality != snap.quality
            || self.pending.force_8bit_quantization != snap.force_8bit_quantization
        {
            if let Some(quality) = self.pending.quality {
                unsafe {
                    ffi::jpeg_set_quality(
                        &mut self.cinfo,
                        quality as c_int,
                        boolean::from(force_8bit),
                    );
                }
            }
        }

        if defaults_called || self.pending.luma_qtable != snap.luma_qtable
            || self.pending.force_8bit_quantization != snap.force_8bit_quantization
        {
            if let Some(ref qtable) = self.pending.luma_qtable {
                unsafe {
                    ffi::jpeg_add_quant_table(
                        &mut self.cinfo,
                        0,
                        qtable.as_ptr().cast(),
                        100,
                        boolean::from(force_8bit) as c_int,
                    );
                }
            }
        }

        if defaults_called || self.pending.chroma_qtable != snap.chroma_qtable
            || self.pending.force_8bit_quantization != snap.force_8bit_quantization
        {
            if let Some(ref qtable) = self.pending.chroma_qtable {
                unsafe {
                    ffi::jpeg_add_quant_table(
                        &mut self.cinfo,
                        1,
                        qtable.as_ptr().cast(),
                        100,
                        boolean::from(force_8bit) as c_int,
                    );
                }
            }
        }

        // Smoothing (reset to 0 by jpeg_set_defaults)
        if defaults_called || self.pending.smoothing_factor != snap.smoothing_factor {
            if let Some(factor) = self.pending.smoothing_factor {
                self.cinfo.smoothing_factor = factor as c_int;
            }
        }

        // Pixel density
        if defaults_called || self.pending.pixel_density != snap.pixel_density {
            if let Some(density) = self.pending.pixel_density {
                self.cinfo.density_unit = density.unit as u8;
                self.cinfo.X_density = density.x;
                self.cinfo.Y_density = density.y;
            }
        }

        // Coding optimization
        if defaults_called || self.pending.optimize_coding != snap.optimize_coding {
            if let Some(opt) = self.pending.optimize_coding {
                self.cinfo.optimize_coding = boolean::from(opt);
            }
        }

        // Scan optimization (mozjpeg extension)
        if defaults_called || self.pending.optimize_scans != snap.optimize_scans {
            if let Some(opt) = self.pending.optimize_scans {
                unsafe {
                    ffi::jpeg_c_set_bool_param(
                        &mut self.cinfo,
                        J_BOOLEAN_PARAM::JBOOLEAN_OPTIMIZE_SCANS,
                        boolean::from(opt),
                    );
                }
                if !opt {
                    self.cinfo.scan_info = ptr::null();
                }
            }
        }

        // Trellis scans (mozjpeg extension)
        if defaults_called || self.pending.use_scans_in_trellis != snap.use_scans_in_trellis {
            if let Some(opt) = self.pending.use_scans_in_trellis {
                unsafe {
                    ffi::jpeg_c_set_bool_param(
                        &mut self.cinfo,
                        J_BOOLEAN_PARAM::JBOOLEAN_USE_SCANS_IN_TRELLIS,
                        boolean::from(opt),
                    );
                }
            }
        }

        // Progressive mode (reset by jpeg_set_defaults, can only be turned on)
        if self.pending.progressive_mode && (defaults_called || !snap.progressive_mode) {
            unsafe {
                ffi::jpeg_simple_progression(&mut self.cinfo);
            }
        }

        // Raw data input
        if defaults_called || self.pending.raw_data_in != snap.raw_data_in {
            if let Some(opt) = self.pending.raw_data_in {
                self.cinfo.raw_data_in = boolean::from(opt);
            }
        }

        // Subsampling factors (reset by jpeg_set_defaults)
        if defaults_called || self.pending.subsampling != snap.subsampling {
            if let Some(ref factors) = self.pending.subsampling {
                let num_components = self.cinfo.num_components as usize;
                for (i, &(h, v)) in factors.iter().enumerate() {
                    if i < num_components {
                        unsafe {
                            (*self.cinfo.comp_info.add(i)).h_samp_factor = h;
                            (*self.cinfo.comp_info.add(i)).v_samp_factor = v;
                        }
                    }
                }
            }
        }

        // === Phase 6: Run user callbacks LAST ===
        for callback in std::mem::take(&mut self.raw_callbacks) {
            callback(&mut self.cinfo);
        }

        // === Update snapshot to reflect what we just applied ===
        self.applied_snapshot = Some(self.pending.clone());
    }

    /// Settings can't be changed after this call. Returns a `CompressStarted` struct that will handle the rest of the writing.
    ///
    /// All pending configuration is applied in the correct order before compression starts.
    ///
    /// ## Panics
    ///
    /// It may panic, like all functions of this library.
    pub fn start_compress<W: io::Write>(mut self, writer: W) -> io::Result<CompressStarted<W>> {
        // Apply all pending configuration in the correct order
        self.apply_pending_config();

        // Validate dimensions
        let width = self.cinfo.image_width;
        let height = self.cinfo.image_height;
        if width == 0 || height == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "invalid dimensions: {}x{} (both must be > 0)",
                    width, height
                ),
            ));
        }
        // JPEG standard max is 65535, but libjpeg may have lower limits
        const JPEG_MAX_DIMENSION: u32 = 65535;
        if width > JPEG_MAX_DIMENSION || height > JPEG_MAX_DIMENSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "dimensions {}x{} exceed JPEG maximum of {}",
                    width, height, JPEG_MAX_DIMENSION
                ),
            ));
        }

        // Validate sampling factors
        for (i, comp) in self.components().iter().enumerate() {
            if comp.h_samp_factor <= 0 || comp.v_samp_factor <= 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "component {} has invalid sampling factors: h={}, v={} (must be > 0)",
                        i, comp.h_samp_factor, comp.v_samp_factor
                    ),
                ));
            }
            // libjpeg supports max sampling factor of 4
            if comp.h_samp_factor > 4 || comp.v_samp_factor > 4 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "component {} has sampling factors h={}, v={} (max is 4)",
                        i, comp.h_samp_factor, comp.v_samp_factor
                    ),
                ));
            }
        }

        if !self.components().iter().any(|c| c.h_samp_factor == 1) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "at least one h_samp_factor must be 1",
            ));
        }
        if !self.components().iter().any(|c| c.v_samp_factor == 1) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "at least one v_samp_factor must be 1",
            ));
        }

        // 1bpp, rounded to 4K page
        let expected_file_size =
            (self.cinfo.image_width as usize * self.cinfo.image_height as usize / 8 + 4095) & !4095;
        let write_buffer_capacity = expected_file_size.clamp(1 << 12, 1 << 16);

        let mut started = CompressStarted {
            compress: self,
            dest_mgr: DestinationMgr::new(writer, write_buffer_capacity),
        };
        unsafe {
            started.compress.cinfo.dest = started.dest_mgr.iface_c_ptr();
            ffi::jpeg_start_compress(&mut started.compress.cinfo, boolean::from(true));
        }
        Ok(started)
    }
}

impl<W> CompressStarted<W> {
    /// Add a marker to compressed file
    ///
    /// Data is max 64KB
    ///
    /// ## Panics
    ///
    /// It may panic, like all functions of this library.
    pub fn write_marker(&mut self, marker: Marker, data: &[u8]) {
        unsafe {
            ffi::jpeg_write_marker(
                &mut self.compress.cinfo,
                marker.into(),
                data.as_ptr(),
                data.len() as c_uint,
            );
        }
    }

    /// Add ICC profile to compressed file
    ///
    /// ## Panics
    ///
    /// It may panic, like all functions of this library.
    pub fn write_icc_profile(&mut self, data: &[u8]) {
        const OVERHEAD_LEN: usize = 14;
        const MAX_BYTES_IN_MARKER: usize = 65533;
        const MAX_DATA_BYTES_IN_MARKER: usize = MAX_BYTES_IN_MARKER - OVERHEAD_LEN;

        if data.is_empty() {
            fail(&mut self.compress.cinfo.common, ffi::JERR_BUFFER_SIZE);
        }

        let chunks = data.chunks(MAX_DATA_BYTES_IN_MARKER);
        let num_chunks = chunks.len();

        let mut buf = Vec::with_capacity(MAX_BYTES_IN_MARKER.min(data.len() + OVERHEAD_LEN));

        chunks.enumerate().for_each(move |(current_marker, chunk)| {
            buf.clear();
            buf.extend_from_slice(b"ICC_PROFILE\0");
            buf.extend([current_marker as u8, num_chunks as u8]);
            buf.extend_from_slice(chunk);

            self.write_marker(Marker::APP(2), &buf);
        });
    }

    /// Read-only view of component information.
    ///
    /// All configuration has already been applied by `start_compress()`.
    #[must_use]
    pub fn components(&self) -> &[CompInfo] {
        if self.compress.cinfo.comp_info.is_null() {
            return &[];
        }
        unsafe { slice::from_raw_parts(self.compress.cinfo.comp_info, self.compress.cinfo.num_components as usize) }
    }

    fn can_write_more_lines(&self) -> bool {
        self.compress.cinfo.next_scanline < self.compress.cinfo.image_height
    }
}

impl Compress {
    /// Modify components via a callback that runs LAST, after all other configuration.
    ///
    /// The callback is executed at [`start_compress()`](Self::start_compress) time,
    /// after `jpeg_set_defaults()` has been called and all pending settings applied.
    /// This ensures your modifications won't be reset.
    ///
    /// # Example
    ///
    /// ```
    /// use mozjpeg::{Compress, ColorSpace};
    ///
    /// let mut comp = Compress::new(ColorSpace::JCS_YCbCr);
    /// comp.set_size(64, 64);
    ///
    /// comp.mutate_components_last(|components| {
    ///     // Set 4:2:0 subsampling
    ///     if components.len() >= 3 {
    ///         components[0].h_samp_factor = 2;
    ///         components[0].v_samp_factor = 2;
    ///         components[1].h_samp_factor = 1;
    ///         components[1].v_samp_factor = 1;
    ///         components[2].h_samp_factor = 1;
    ///         components[2].v_samp_factor = 1;
    ///     }
    /// });
    /// ```
    pub fn mutate_components_last<F>(&mut self, f: F)
    where
        F: FnOnce(&mut [CompInfo]) + 'static,
    {
        self.raw_callbacks.push(Box::new(move |cinfo| {
            if !cinfo.comp_info.is_null() && cinfo.num_components > 0 {
                let components = unsafe {
                    slice::from_raw_parts_mut(cinfo.comp_info, cinfo.num_components as usize)
                };
                f(components);
            }
        }));
    }

    /// Access raw `jpeg_compress_struct` via a callback that runs LAST, after all other configuration.
    ///
    /// The callback is executed at [`start_compress()`](Self::start_compress) time,
    /// after `jpeg_set_defaults()` has been called and all pending settings applied.
    /// This ensures your modifications won't be reset.
    ///
    /// # Safety
    ///
    /// While this method is safe to call, the callback receives raw access to libjpeg's
    /// internal state. Invalid modifications may cause undefined behavior during compression.
    ///
    /// # Example
    ///
    /// ```
    /// use mozjpeg::{Compress, ColorSpace};
    ///
    /// let mut comp = Compress::new(ColorSpace::JCS_RGB);
    /// comp.set_size(64, 64);
    ///
    /// comp.mutate_cinfo_last(|cinfo| {
    ///     // Direct cinfo access for advanced use cases
    ///     cinfo.smoothing_factor = 50;
    /// });
    /// ```
    pub fn mutate_cinfo_last<F>(&mut self, f: F)
    where
        F: FnOnce(&mut jpeg_compress_struct) + 'static,
    {
        self.raw_callbacks.push(Box::new(f));
    }

    /// Expose components for modification, e.g. to set chroma subsampling.
    ///
    /// # Deprecated
    ///
    /// Use [`mutate_components_last()`](Self::mutate_components_last) instead for order-independent configuration.
    ///
    /// This method applies all pending configuration before returning the reference.
    /// Any setter calls made AFTER calling this method will have no effect.
    #[deprecated(
        since = "0.11.0",
        note = "Use mutate_components_last() for order-independent configuration"
    )]
    pub fn components_mut(&mut self) -> &mut [CompInfo] {
        // Apply pending config so the caller sees the computed state
        self.apply_pending_config();
        if self.cinfo.comp_info.is_null() {
            return &mut [];
        }
        unsafe {
            slice::from_raw_parts_mut(self.cinfo.comp_info, self.cinfo.num_components as usize)
        }
    }

    /// Read-only view of component information.
    ///
    /// Applies pending configuration first so the returned values reflect
    /// all setter calls made so far.
    #[must_use]
    pub fn components(&mut self) -> &[CompInfo] {
        self.apply_pending_config();
        if self.cinfo.comp_info.is_null() {
            return &[];
        }
        unsafe { slice::from_raw_parts(self.cinfo.comp_info, self.cinfo.num_components as usize) }
    }

    /// Access to cinfo for inspecting configuration state.
    ///
    /// Applies pending configuration first so the returned values reflect
    /// all setter calls made so far.
    #[doc(hidden)]
    pub fn cinfo(&mut self) -> &ffi::jpeg_compress_struct {
        self.apply_pending_config();
        &self.cinfo
    }

    /// Internal mutable access to cinfo.
    ///
    /// # Deprecated
    ///
    /// Use [`mutate_cinfo_last()`](Self::mutate_cinfo_last) instead for order-independent configuration.
    ///
    /// This method applies all pending configuration before returning the reference.
    /// Any setter calls made AFTER calling this method will have no effect.
    #[doc(hidden)]
    #[deprecated(
        since = "0.11.0",
        note = "Use mutate_cinfo_last() for order-independent configuration"
    )]
    pub fn cinfo_mut(&mut self) -> &mut ffi::jpeg_compress_struct {
        self.apply_pending_config();
        &mut self.cinfo
    }
}

impl<W> CompressStarted<W> {
    /// Returns Ok(()) if all lines in `image_src` (not necessarily all lines of the image) were written
    ///
    /// ## Panics
    ///
    /// It may panic, like all functions of this library.
    pub fn write_scanlines(&mut self, image_src: &[u8]) -> io::Result<()> {
        if self.compress.cinfo.raw_data_in != 0
            || self.compress.cinfo.input_components <= 0
            || self.compress.cinfo.image_width == 0
        {
            return Err(io::ErrorKind::InvalidInput.into());
        }

        let byte_width = self.compress.cinfo.image_width as usize
            * self.compress.cinfo.input_components as usize;
        self.write_scanlines_strided(image_src, byte_width)
    }

    /// Write scanlines with custom stride (bytes per row)
    ///
    /// Use this when your pixel data has padding/alignment between rows.
    /// `stride` is the number of bytes from the start of one row to the start of the next.
    ///
    /// ## Panics
    ///
    /// It may panic, like all functions of this library.
    pub fn write_scanlines_strided(&mut self, image_src: &[u8], stride: usize) -> io::Result<()> {
        if self.compress.cinfo.raw_data_in != 0
            || self.compress.cinfo.input_components <= 0
            || self.compress.cinfo.image_width == 0
        {
            return Err(io::ErrorKind::InvalidInput.into());
        }

        let byte_width = self.compress.cinfo.image_width as usize
            * self.compress.cinfo.input_components as usize;

        // Validate stride
        if stride < byte_width {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("stride ({stride}) must be >= byte_width ({byte_width})"),
            ));
        }

        // Process rows in chunks of MAX_MCU_HEIGHT
        let mut offset = 0;
        while offset < image_src.len() {
            let mut row_pointers = ArrayVec::<_, MAX_MCU_HEIGHT>::new();

            // Collect up to MAX_MCU_HEIGHT row pointers
            for _ in 0..MAX_MCU_HEIGHT {
                if offset + byte_width > image_src.len() {
                    break;
                }
                row_pointers.push(image_src[offset..].as_ptr());
                offset += stride;
            }

            if row_pointers.is_empty() {
                break;
            }

            // Write the rows
            let mut rows_left = row_pointers.len() as u32;
            let mut row_pointers = row_pointers.as_ptr();
            while rows_left > 0 {
                unsafe {
                    let rows_written = ffi::jpeg_write_scanlines(
                        &mut self.compress.cinfo,
                        row_pointers,
                        rows_left,
                    );
                    debug_assert!(rows_left >= rows_written);
                    if rows_written == 0 {
                        return Err(io::ErrorKind::UnexpectedEof.into());
                    }
                    rows_left -= rows_written;
                    row_pointers = row_pointers.add(rows_written as usize);
                }
            }
        }
        Ok(())
    }

    /// Advanced. Only possible after `set_raw_data_in()`.
    /// Write YCbCr blocks pixels instead of usual color space
    ///
    /// See `raw_data_in` in libjpeg docs
    ///
    /// ## Panic
    ///
    /// Panics if raw write wasn't enabled
    #[track_caller]
    pub fn write_raw_data(&mut self, image_src: &[&[u8]]) -> bool {
        if 0 == self.compress.cinfo.raw_data_in {
            panic!("Raw data not set");
        }

        let mcu_height = self.compress.cinfo.max_v_samp_factor as usize * DCTSIZE;
        if mcu_height > MAX_MCU_HEIGHT {
            panic!("Subsampling factor too large");
        }
        assert!(mcu_height > 0);

        let num_components = self.components().len();
        if num_components > MAX_COMPONENTS || num_components > image_src.len() {
            panic!(
                "Too many components: declared {}, got {}",
                num_components,
                image_src.len()
            );
        }

        for (ci, comp_info) in self.components().iter().enumerate() {
            if comp_info.row_stride() * comp_info.col_stride() > image_src[ci].len() {
                panic!(
                    "Bitmap too small. Expected {}x{}, got {}",
                    comp_info.row_stride(),
                    comp_info.col_stride(),
                    image_src[ci].len()
                );
            }
        }

        let mut start_row = self.compress.cinfo.next_scanline as usize;
        while self.can_write_more_lines() {
            unsafe {
                let mut row_ptrs = [[ptr::null::<u8>(); MAX_MCU_HEIGHT]; MAX_COMPONENTS];

                for ((comp_info, &image_src), comp_row_ptrs) in self
                    .components()
                    .iter()
                    .zip(image_src)
                    .zip(row_ptrs.iter_mut())
                {
                    let row_stride = comp_info.row_stride();

                    let input_height = image_src.len() / row_stride;

                    let comp_start_row = start_row * comp_info.v_samp_factor as usize
                        / self.compress.cinfo.max_v_samp_factor as usize;
                    let comp_height = min(
                        input_height - comp_start_row,
                        DCTSIZE * comp_info.v_samp_factor as usize,
                    );
                    assert!(comp_height >= 8);

                    // row_ptrs were initialized to null
                    for (src_row, row_ptr) in image_src
                        .chunks_exact(row_stride)
                        .skip(comp_start_row)
                        .take(comp_height)
                        .zip(comp_row_ptrs.iter_mut())
                    {
                        *row_ptr = src_row.as_ptr();
                    }
                }

                let comp_ptrs: [*const *const u8; MAX_COMPONENTS] =
                    std::array::from_fn(|ci| row_ptrs[ci].as_ptr());
                let rows_written = ffi::jpeg_write_raw_data(
                    &mut self.compress.cinfo,
                    comp_ptrs.as_ptr(),
                    mcu_height as u32,
                ) as usize;
                if 0 == rows_written {
                    return false;
                }
                start_row += rows_written;
            }
        }
        true
    }
}

impl Compress {
    /// Set color space of JPEG being written, different from input color space
    ///
    /// This setting is applied at [`start_compress()`](Self::start_compress) time,
    /// so you can call configuration methods in any order.
    ///
    /// See `jpeg_set_colorspace` in libjpeg docs.
    pub fn set_color_space(&mut self, color_space: ColorSpace) {
        self.pending.output_colorspace = Some(color_space);
    }

    /// Image size of the input.
    ///
    /// This setting is applied at [`start_compress()`](Self::start_compress) time,
    /// so you can call configuration methods in any order.
    pub fn set_size(&mut self, width: usize, height: usize) {
        self.pending.width = Some(width as u32);
        self.pending.height = Some(height as u32);
    }

    /// libjpeg's `input_gamma` = image gamma of input image
    #[deprecated(note = "it doesn't do anything")]
    pub fn set_gamma(&mut self, gamma: f64) {
        self.cinfo.input_gamma = gamma;
    }

    /// Sets pixel density of an image in the JFIF APP0 segment[^note].
    /// If this method is not called, the resulting JPEG will have a default
    /// pixel aspect ratio of 1x1.
    ///
    /// [^note]: This method is not related to EXIF-based intrinsic image sizing,
    /// and does not affect rendering in browsers.
    ///
    /// This setting is applied at [`start_compress()`](Self::start_compress) time,
    /// so you can call configuration methods in any order.
    pub fn set_pixel_density(&mut self, density: PixelDensity) {
        self.pending.pixel_density = Some(density);
    }

    /// If true, it will use MozJPEG's scan optimization. Makes progressive image files smaller.
    ///
    /// This setting is applied at [`start_compress()`](Self::start_compress) time,
    /// so you can call configuration methods in any order.
    pub fn set_optimize_scans(&mut self, opt: bool) {
        self.pending.optimize_scans = Some(opt);
    }

    /// If 1-100 (non-zero), it will use MozJPEG's smoothing.
    ///
    /// This setting is applied at [`start_compress()`](Self::start_compress) time,
    /// so you can call configuration methods in any order.
    pub fn set_smoothing_factor(&mut self, smoothing_factor: u8) {
        self.pending.smoothing_factor = Some(smoothing_factor);
    }

    /// Set to `false` to make files larger for no reason.
    ///
    /// This setting is applied at [`start_compress()`](Self::start_compress) time,
    /// so you can call configuration methods in any order.
    pub fn set_optimize_coding(&mut self, opt: bool) {
        self.pending.optimize_coding = Some(opt);
    }

    /// Specifies whether multiple scans should be considered during trellis
    /// quantization.
    ///
    /// This setting is applied at [`start_compress()`](Self::start_compress) time,
    /// so you can call configuration methods in any order.
    pub fn set_use_scans_in_trellis(&mut self, opt: bool) {
        self.pending.use_scans_in_trellis = Some(opt);
    }

    /// You can only turn it on.
    ///
    /// This setting is applied at [`start_compress()`](Self::start_compress) time,
    /// so you can call configuration methods in any order.
    pub fn set_progressive_mode(&mut self) {
        self.pending.progressive_mode = true;
    }

    /// One scan for all components looks best. Other options may flash grayscale or green images.
    ///
    /// This setting is applied at [`start_compress()`](Self::start_compress) time,
    /// so you can call configuration methods in any order.
    pub fn set_scan_optimization_mode(&mut self, mode: ScanMode) {
        self.pending.scan_mode = Some(mode);
    }

    /// Reset to libjpeg v6 settings.
    ///
    /// It gives files identical with libjpeg-turbo.
    ///
    /// This setting is applied at [`start_compress()`](Self::start_compress) time,
    /// so you can call configuration methods in any order.
    pub fn set_fastest_defaults(&mut self) {
        self.pending.fastest_defaults = true;
    }

    /// Advanced. See `raw_data_in` in libjpeg docs.
    ///
    /// This setting is applied at [`start_compress()`](Self::start_compress) time,
    /// so you can call configuration methods in any order.
    pub fn set_raw_data_in(&mut self, opt: bool) {
        self.pending.raw_data_in = Some(opt);
    }

    /// Set image quality. Values 60-80 are recommended.
    ///
    /// This setting is applied at [`start_compress()`](Self::start_compress) time,
    /// so you can call configuration methods in any order.
    pub fn set_quality(&mut self, quality: f32) {
        self.pending.quality = Some(quality.round() as i32);
    }

    /// When `true`, quantization table values are clamped to 1-255 (8-bit DQT precision).
    /// When `false` (the default), values can go up to 32767 (16-bit DQT precision).
    ///
    /// Applies to both `set_quality()` and custom quantization tables.
    ///
    /// This setting is applied at [`start_compress()`](Self::start_compress) time,
    /// so you can call configuration methods in any order.
    pub fn set_force_8bit_quantization(&mut self, force_8bit: bool) {
        self.pending.force_8bit_quantization = force_8bit;
    }

    /// Instead of quality setting, use a specific quantization table.
    ///
    /// This setting is applied at [`start_compress()`](Self::start_compress) time,
    /// so you can call configuration methods in any order.
    pub fn set_luma_qtable(&mut self, qtable: &QTable) {
        self.pending.luma_qtable = Some(qtable.clone());
    }

    /// Instead of quality setting, use a specific quantization table for color.
    ///
    /// This setting is applied at [`start_compress()`](Self::start_compress) time,
    /// so you can call configuration methods in any order.
    pub fn set_chroma_qtable(&mut self, qtable: &QTable) {
        self.pending.chroma_qtable = Some(qtable.clone());
    }



    /// Sets chroma subsampling, separately for Cb and Cr channels.
    /// Instead of setting samples per pixel, like in `cinfo`'s `x_samp_factor`,
    /// it sets size of chroma "pixels" per luma pixel.
    ///
    /// * `(1,1), (1,1)` == 4:4:4
    /// * `(2,1), (2,1)` == 4:2:2
    /// * `(2,2), (2,2)` == 4:2:0
    ///
    /// This setting is applied at [`start_compress()`](Self::start_compress) time,
    /// so you can call configuration methods in any order.
    pub fn set_chroma_sampling_pixel_sizes(&mut self, cb: (u8, u8), cr: (u8, u8)) {
        let max_sampling_h = cb.0.max(cr.0);
        let max_sampling_v = cb.1.max(cr.1);

        let px_sizes = [(1, 1), cb, cr];
        let factors: Vec<(i32, i32)> = px_sizes
            .iter()
            .map(|(h, v)| ((max_sampling_h / h) as i32, (max_sampling_v / v) as i32))
            .collect();
        self.pending.subsampling = Some(factors);
    }
}

impl<W: io::Write> CompressStarted<W> {
    /// Finalize compression.
    /// In case of progressive files, this may actually start processing.
    ///
    /// ## Panics
    ///
    /// It may panic, like all functions of this library.
    #[inline]
    pub fn finish(mut self) -> io::Result<W> {
        unsafe {
            ffi::jpeg_finish_compress(&mut self.compress.cinfo);
        }
        self.compress.cinfo.dest = ptr::null_mut();
        drop(self.compress);
        Ok(self.dest_mgr.into_inner())
    }

    #[doc(hidden)]
    #[deprecated(note = "use finish(); it now returns a writer given to start_compress()")]
    pub fn finish_compress(self) -> io::Result<W> {
        self.finish()
    }

    /// Give up writing, return incomplete result
    #[cold]
    pub fn abort(mut self) -> W {
        self.compress.cinfo.dest = ptr::null_mut();
        self.dest_mgr.into_inner()
    }
}

impl Drop for Compress {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            self.cinfo.dest = ptr::null_mut();
            ffi::jpeg_destroy_compress(&mut self.cinfo);
            // ErrorMgr is destroyed after cinfo can no longer reference it
            let _ = Box::from_raw(self.own_err);
        }
    }
}

#[test]
fn write_mem() {
    let mut comp = Compress::new(ColorSpace::JCS_YCbCr);

    assert_eq!(3, comp.components().len());

    comp.set_size(17, 33);

    #[allow(deprecated)]
    {
        comp.set_gamma(1.0);
    }

    comp.set_progressive_mode();
    comp.set_scan_optimization_mode(ScanMode::AllComponentsTogether);

    comp.set_raw_data_in(true);

    comp.set_quality(88.);

    // With lazy config, subsampling is applied at start_compress time
    comp.set_chroma_sampling_pixel_sizes((2, 2), (2, 2));

    let mut started = comp.start_compress(Vec::new()).unwrap();

    // Now we can check the applied values
    for (c, samp) in started.components().iter().zip([2, 1, 1]) {
        assert_eq!(c.v_samp_factor, samp);
        assert_eq!(c.h_samp_factor, samp);
    }

    started.write_marker(Marker::APP(2), b"Hello World");

    assert_eq!(24, started.components()[0].row_stride());
    assert_eq!(40, started.components()[0].col_stride());
    assert_eq!(16, started.components()[1].row_stride());
    assert_eq!(24, started.components()[1].col_stride());
    assert_eq!(16, started.components()[2].row_stride());
    assert_eq!(24, started.components()[2].col_stride());

    let bitmaps = started
        .components()
        .iter()
        .map(|c| vec![128u8; c.row_stride() * c.col_stride()])
        .collect::<Vec<_>>();

    assert!(started.write_raw_data(&bitmaps.iter().map(|c| &c[..]).collect::<Vec<_>>()));

    started.finish().unwrap();
}

#[test]
fn convert_colorspace() {
    let mut comp = Compress::new(ColorSpace::JCS_RGB);
    comp.set_color_space(ColorSpace::JCS_GRAYSCALE);

    // components() applies pending config, so colorspace change is visible immediately
    assert_eq!(1, comp.components().len()); // Grayscale after pending config applied

    comp.set_size(33, 15);
    comp.set_quality(44.);

    let mut started = comp.start_compress(Vec::new()).unwrap();

    // After start_compress(), the colorspace is applied
    assert_eq!(1, started.components().len()); // Now grayscale

    let scanlines = vec![127u8; 33 * 15 * 3];
    started.write_scanlines(&scanlines).unwrap();

    let res = started.finish().unwrap();
    assert!(!res.is_empty());
}

// === Tests for deprecated methods and incremental config application ===

#[test]
#[allow(deprecated)]
fn deprecated_components_mut_applies_pending_config() {
    // Test that components_mut() applies pending config before returning
    let mut comp = Compress::new(ColorSpace::JCS_RGB);
    comp.set_size(64, 64);
    comp.set_color_space(ColorSpace::JCS_YCbCr);
    comp.set_quality(85.0);
    comp.set_smoothing_factor(50);

    // Before components_mut(), cinfo hasn't been updated
    assert_eq!(3, comp.components().len()); // Still RGB components

    // components_mut() triggers apply_pending_config()
    let components = comp.components_mut();
    assert_eq!(3, components.len()); // Now YCbCr (still 3 components)

    // Verify smoothing was applied
    assert_eq!(50, comp.cinfo.smoothing_factor);
}

#[test]
#[allow(deprecated)]
fn deprecated_components_mut_incremental_updates() {
    // Test that settings added AFTER components_mut() are still applied at start_compress()
    let mut comp = Compress::new(ColorSpace::JCS_RGB);
    comp.set_size(64, 64);
    comp.set_quality(85.0);

    // First call to components_mut() applies quality=85
    let _components = comp.components_mut();

    // Add more settings after components_mut()
    comp.set_smoothing_factor(75);
    comp.set_optimize_coding(true);

    // These should be applied at start_compress()
    let pixels: Vec<u8> = vec![128u8; 64 * 64 * 3];
    let mut started = comp.start_compress(Vec::new()).unwrap();

    // Verify the new settings were applied
    assert_eq!(75, started.compress.cinfo.smoothing_factor);
    assert_ne!(0, started.compress.cinfo.optimize_coding);

    started.write_scanlines(&pixels).unwrap();
    started.finish().unwrap();
}

#[test]
#[allow(deprecated)]
fn deprecated_components_mut_multiple_calls() {
    // Test multiple calls to components_mut() with settings in between
    let mut comp = Compress::new(ColorSpace::JCS_RGB);
    comp.set_size(64, 64);

    // Set quality, then access components
    comp.set_quality(70.0);
    let _c1 = comp.components_mut();

    // Add smoothing, then access components again
    comp.set_smoothing_factor(30);
    let _c2 = comp.components_mut();
    assert_eq!(30, comp.cinfo.smoothing_factor);

    // Add more settings
    comp.set_optimize_coding(true);

    // Final application at start_compress
    let pixels: Vec<u8> = vec![128u8; 64 * 64 * 3];
    let mut started = comp.start_compress(Vec::new()).unwrap();

    assert_eq!(30, started.compress.cinfo.smoothing_factor);
    assert_ne!(0, started.compress.cinfo.optimize_coding);

    started.write_scanlines(&pixels).unwrap();
    started.finish().unwrap();
}

#[test]
fn mutate_components_last_runs_after_all_config() {
    // Test that mutate_components_last callback runs after all other config
    let mut comp = Compress::new(ColorSpace::JCS_RGB);
    comp.set_size(64, 64);
    comp.set_color_space(ColorSpace::JCS_YCbCr);
    comp.set_scan_optimization_mode(ScanMode::Auto); // Would reset settings

    // This callback should see the fully configured state
    comp.mutate_components_last(|components| {
        // Verify we have 3 YCbCr components
        assert_eq!(3, components.len());
        // Set custom subsampling
        components[0].h_samp_factor = 2;
        components[0].v_samp_factor = 2;
        components[1].h_samp_factor = 1;
        components[1].v_samp_factor = 1;
        components[2].h_samp_factor = 1;
        components[2].v_samp_factor = 1;
    });

    let pixels: Vec<u8> = vec![128u8; 64 * 64 * 3];
    let mut started = comp.start_compress(Vec::new()).unwrap();

    // Verify the callback's changes were applied
    let comps = started.components();
    assert_eq!(2, comps[0].h_samp_factor);
    assert_eq!(2, comps[0].v_samp_factor);
    assert_eq!(1, comps[1].h_samp_factor);

    started.write_scanlines(&pixels).unwrap();
    started.finish().unwrap();
}

#[test]
fn mutate_cinfo_last_runs_after_all_config() {
    // Test that mutate_cinfo_last callback runs after all other config
    let mut comp = Compress::new(ColorSpace::JCS_RGB);
    comp.set_size(64, 64);
    comp.set_quality(85.0);
    comp.set_smoothing_factor(25);
    comp.set_scan_optimization_mode(ScanMode::Auto);

    // This callback can override anything
    comp.mutate_cinfo_last(|cinfo| {
        // Override smoothing to a different value
        cinfo.smoothing_factor = 99;
    });

    let pixels: Vec<u8> = vec![128u8; 64 * 64 * 3];
    let mut started = comp.start_compress(Vec::new()).unwrap();

    // The callback ran last and overrode the smoothing
    assert_eq!(99, started.compress.cinfo.smoothing_factor);

    started.write_scanlines(&pixels).unwrap();
    started.finish().unwrap();
}

#[test]
fn lazy_config_order_independence_comprehensive() {
    // Comprehensive test that all settings work regardless of order
    let pixels: Vec<u8> = (0..64 * 64 * 3).map(|i| (i % 256) as u8).collect();

    // Helper to encode with a specific order of settings
    let encode = |order: &str| -> Vec<u8> {
        let mut comp = Compress::new(ColorSpace::JCS_RGB);

        match order {
            "normal" => {
                comp.set_size(64, 64);
                comp.set_color_space(ColorSpace::JCS_YCbCr);
                comp.set_quality(75.0);
                comp.set_smoothing_factor(20);
                comp.set_chroma_sampling_pixel_sizes((2, 2), (2, 2));
                comp.set_scan_optimization_mode(ScanMode::Auto);
            }
            "reversed" => {
                comp.set_scan_optimization_mode(ScanMode::Auto);
                comp.set_chroma_sampling_pixel_sizes((2, 2), (2, 2));
                comp.set_smoothing_factor(20);
                comp.set_quality(75.0);
                comp.set_color_space(ColorSpace::JCS_YCbCr);
                comp.set_size(64, 64);
            }
            "interleaved" => {
                comp.set_scan_optimization_mode(ScanMode::Auto);
                comp.set_size(64, 64);
                comp.set_smoothing_factor(20);
                comp.set_color_space(ColorSpace::JCS_YCbCr);
                comp.set_chroma_sampling_pixel_sizes((2, 2), (2, 2));
                comp.set_quality(75.0);
            }
            _ => panic!("Unknown order"),
        }

        let mut started = comp.start_compress(Vec::new()).unwrap();
        started.write_scanlines(&pixels).unwrap();
        started.finish().unwrap()
    };

    let normal = encode("normal");
    let reversed = encode("reversed");
    let interleaved = encode("interleaved");

    // All orderings should produce identical output
    assert_eq!(normal, reversed, "normal vs reversed should match");
    assert_eq!(normal, interleaved, "normal vs interleaved should match");
}

#[test]
#[allow(deprecated)]
fn deprecated_then_new_api_works() {
    // Test mixing deprecated components_mut() with new mutate_components_last()
    let mut comp = Compress::new(ColorSpace::JCS_RGB);
    comp.set_size(64, 64);
    comp.set_quality(85.0);

    // Use deprecated API first
    {
        let components = comp.components_mut();
        components[0].h_samp_factor = 2;
    }

    // Then use new API - should still work
    comp.set_smoothing_factor(40);
    comp.mutate_components_last(|components| {
        // This runs after everything else
        components[0].v_samp_factor = 2;
    });

    let pixels: Vec<u8> = vec![128u8; 64 * 64 * 3];
    let mut started = comp.start_compress(Vec::new()).unwrap();

    // Verify both modifications were applied
    let comps = started.components();
    assert_eq!(2, comps[0].h_samp_factor); // From deprecated API
    assert_eq!(2, comps[0].v_samp_factor); // From new API callback
    assert_eq!(40, started.compress.cinfo.smoothing_factor);

    started.write_scanlines(&pixels).unwrap();
    started.finish().unwrap();
}

#[test]
fn scan_mode_does_not_reset_quality() {
    // This was the core bug: set_scan_optimization_mode() calls jpeg_set_defaults()
    // which would reset quality. With lazy config, order shouldn't matter.
    let pixels: Vec<u8> = vec![128u8; 64 * 64 * 3];

    // Order 1: quality first, then scan mode
    let mut comp1 = Compress::new(ColorSpace::JCS_RGB);
    comp1.set_size(64, 64);
    comp1.set_quality(50.0);
    comp1.set_scan_optimization_mode(ScanMode::Auto);
    let mut started1 = comp1.start_compress(Vec::new()).unwrap();
    started1.write_scanlines(&pixels).unwrap();
    let result1 = started1.finish().unwrap();

    // Order 2: scan mode first, then quality
    let mut comp2 = Compress::new(ColorSpace::JCS_RGB);
    comp2.set_size(64, 64);
    comp2.set_scan_optimization_mode(ScanMode::Auto);
    comp2.set_quality(50.0);
    let mut started2 = comp2.start_compress(Vec::new()).unwrap();
    started2.write_scanlines(&pixels).unwrap();
    let result2 = started2.finish().unwrap();

    // Both should produce identical output
    assert_eq!(
        result1, result2,
        "Order of set_quality and set_scan_optimization_mode should not matter"
    );
}

#[test]
fn scan_mode_does_not_reset_smoothing() {
    let pixels: Vec<u8> = vec![128u8; 64 * 64 * 3];

    // Smoothing first, then scan mode
    let mut comp1 = Compress::new(ColorSpace::JCS_RGB);
    comp1.set_size(64, 64);
    comp1.set_smoothing_factor(80);
    comp1.set_scan_optimization_mode(ScanMode::Auto);
    let mut started1 = comp1.start_compress(Vec::new()).unwrap();
    assert_eq!(80, started1.compress.cinfo.smoothing_factor);
    started1.write_scanlines(&pixels).unwrap();
    let result1 = started1.finish().unwrap();

    // Scan mode first, then smoothing
    let mut comp2 = Compress::new(ColorSpace::JCS_RGB);
    comp2.set_size(64, 64);
    comp2.set_scan_optimization_mode(ScanMode::Auto);
    comp2.set_smoothing_factor(80);
    let mut started2 = comp2.start_compress(Vec::new()).unwrap();
    assert_eq!(80, started2.compress.cinfo.smoothing_factor);
    started2.write_scanlines(&pixels).unwrap();
    let result2 = started2.finish().unwrap();

    assert_eq!(result1, result2);
}

#[test]
fn fastest_defaults_does_not_reset_settings() {
    let pixels: Vec<u8> = vec![128u8; 64 * 64 * 3];

    // Settings first, then fastest_defaults
    let mut comp1 = Compress::new(ColorSpace::JCS_RGB);
    comp1.set_size(64, 64);
    comp1.set_quality(60.0);
    comp1.set_smoothing_factor(30);
    comp1.set_fastest_defaults();
    let mut started1 = comp1.start_compress(Vec::new()).unwrap();
    assert_eq!(30, started1.compress.cinfo.smoothing_factor);
    started1.write_scanlines(&pixels).unwrap();
    let result1 = started1.finish().unwrap();

    // fastest_defaults first, then settings
    let mut comp2 = Compress::new(ColorSpace::JCS_RGB);
    comp2.set_size(64, 64);
    comp2.set_fastest_defaults();
    comp2.set_quality(60.0);
    comp2.set_smoothing_factor(30);
    let mut started2 = comp2.start_compress(Vec::new()).unwrap();
    assert_eq!(30, started2.compress.cinfo.smoothing_factor);
    started2.write_scanlines(&pixels).unwrap();
    let result2 = started2.finish().unwrap();

    assert_eq!(result1, result2);
}

#[test]
#[allow(deprecated)]
fn raw_changes_preserved_after_deprecated_api() {
    // Test that raw changes made via deprecated components_mut() are preserved
    // when additional buffered settings are applied at start_compress()
    let mut comp = Compress::new(ColorSpace::JCS_RGB);
    comp.set_size(64, 64);
    comp.set_color_space(ColorSpace::JCS_YCbCr);

    // Apply pending config via deprecated API and make raw changes
    {
        let components = comp.components_mut();
        components[0].h_samp_factor = 2;
        components[0].v_samp_factor = 2;
        components[1].h_samp_factor = 1;
        components[1].v_samp_factor = 1;
        components[2].h_samp_factor = 1;
        components[2].v_samp_factor = 1;
    }

    // Add more buffered settings AFTER raw changes
    comp.set_smoothing_factor(50);
    comp.set_quality(75.0);

    let pixels: Vec<u8> = vec![128u8; 64 * 64 * 3];
    let mut started = comp.start_compress(Vec::new()).unwrap();

    // Raw changes should be preserved
    let comps = started.components();
    assert_eq!(
        2, comps[0].h_samp_factor,
        "Raw h_samp_factor change should be preserved"
    );
    assert_eq!(
        2, comps[0].v_samp_factor,
        "Raw v_samp_factor change should be preserved"
    );
    assert_eq!(1, comps[1].h_samp_factor);

    // Buffered settings should also be applied
    assert_eq!(50, started.compress.cinfo.smoothing_factor);

    started.write_scanlines(&pixels).unwrap();
    started.finish().unwrap();
}

#[test]
fn multiple_callbacks_run_in_order() {
    let mut comp = Compress::new(ColorSpace::JCS_RGB);
    comp.set_size(64, 64);

    // Add multiple callbacks - they should run in order
    comp.mutate_cinfo_last(|cinfo| {
        cinfo.smoothing_factor = 10;
    });
    comp.mutate_cinfo_last(|cinfo| {
        // This runs after, so it should override
        cinfo.smoothing_factor = 20;
    });
    comp.mutate_cinfo_last(|cinfo| {
        cinfo.smoothing_factor = 30;
    });

    let pixels: Vec<u8> = vec![128u8; 64 * 64 * 3];
    let mut started = comp.start_compress(Vec::new()).unwrap();

    // Last callback wins
    assert_eq!(30, started.compress.cinfo.smoothing_factor);

    started.write_scanlines(&pixels).unwrap();
    started.finish().unwrap();
}

#[test]
fn pixel_density_preserved_across_scan_mode() {
    use crate::{PixelDensity, PixelDensityUnit};

    let mut comp = Compress::new(ColorSpace::JCS_RGB);
    comp.set_size(64, 64);
    comp.set_pixel_density(PixelDensity {
        unit: PixelDensityUnit::Inches,
        x: 300,
        y: 300,
    });
    comp.set_scan_optimization_mode(ScanMode::Auto);

    let pixels: Vec<u8> = vec![128u8; 64 * 64 * 3];
    let mut started = comp.start_compress(Vec::new()).unwrap();

    assert_eq!(300, started.compress.cinfo.X_density);
    assert_eq!(300, started.compress.cinfo.Y_density);

    started.write_scanlines(&pixels).unwrap();
    started.finish().unwrap();
}

#[test]
fn progressive_mode_preserved_across_scan_mode() {
    let mut comp = Compress::new(ColorSpace::JCS_RGB);
    comp.set_size(64, 64);
    comp.set_progressive_mode();
    comp.set_scan_optimization_mode(ScanMode::Auto);

    let pixels: Vec<u8> = vec![128u8; 64 * 64 * 3];
    let mut started = comp.start_compress(Vec::new()).unwrap();

    // Progressive mode should be set (scan_info not null means progressive)
    assert!(!started.compress.cinfo.scan_info.is_null());

    started.write_scanlines(&pixels).unwrap();
    started.finish().unwrap();
}
