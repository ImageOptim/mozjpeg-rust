use crate::colorspace::ColorSpace;
use crate::colorspace::ColorSpaceExt;
use crate::component::CompInfo;
use crate::component::CompInfoExt;
use crate::errormgr::unwinding_error_mgr;
use crate::errormgr::ErrorMgr;
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
use arrayvec::ArrayVec;
use std::cmp::min;
use std::io;
use std::marker::PhantomPinned;
use std::mem;
use std::os::raw::{c_int, c_uchar, c_uint, c_ulong, c_void};
use std::ptr;
use std::ptr::addr_of_mut;
use std::slice;

const MAX_MCU_HEIGHT: usize = 16;
const MAX_COMPONENTS: usize = 4;

/// Create a new JPEG file from pixels
///
/// Wrapper for `jpeg_compress_struct`
pub struct Compress {
    cinfo: jpeg_compress_struct,

    /// It's `Box<ErrorMgr>`, but `cinfo` references `own_err`,
    /// so I need talismans to ward off nasal demons haunting self-referential structs
    own_err: *mut ErrorMgr,
    _it_is_self_referential: PhantomPinned,
}

#[derive(Copy, Clone)]
pub enum ScanMode {
    AllComponentsTogether = 0,
    ScanPerComponent = 1,
    Auto = 2,
}

pub struct CompressStarted<W> {
    compress: Compress,
    dest_mgr: Box<DestinationMgr<W>>,
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
    pub fn new(color_space: ColorSpace) -> Compress {
        Compress::new_err(unwinding_error_mgr(), color_space)
    }

    /// Use a specific error handler instead of the default unwinding one.
    ///
    /// Note that the error handler must either abort the process or unwind,
    /// it can't gracefully return due to the design of libjpeg.
    ///
    /// `color_space` refers to input color space
    #[must_use]
    pub fn new_err(err: Box<ErrorMgr>, color_space: ColorSpace) -> Compress {
        unsafe {
            let mut newself = Compress {
                cinfo: mem::zeroed(),
                own_err: Box::into_raw(err),
                _it_is_self_referential: PhantomPinned,
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
    pub fn set_mem_dest(&self) {
    }

    /// Settings can't be changed after this call. Returns a `CompressStarted` struct that will handle the rest of the writing.
    ///
    /// ## Panics
    ///
    /// It may panic, like all functions of this library.
    pub fn start_compress<W: io::Write>(self, writer: W) -> io::Result<CompressStarted<W>> {
        if !self.components().iter().any(|c| c.h_samp_factor == 1) { return Err(io::Error::new(io::ErrorKind::InvalidInput, "at least one h_samp_factor must be 1")); }
        if !self.components().iter().any(|c| c.v_samp_factor == 1) { return Err(io::Error::new(io::ErrorKind::InvalidInput, "at least one v_samp_factor must be 1")); }

        // 1bpp, rounded to 4K page
        let expected_file_size = (self.cinfo.image_width as usize * self.cinfo.image_height as usize / 8 + 4095) & !4095;
        let write_buffer_capacity = expected_file_size.clamp(1<<12, 1<<16);

        let mut started = CompressStarted {
            compress: self,
            dest_mgr: Box::new(DestinationMgr::new(writer, write_buffer_capacity)),
        };
        unsafe {
            let dest_ptr = addr_of_mut!(started.dest_mgr.as_mut().iface);
            started.compress.cinfo.dest = dest_ptr;
            ffi::jpeg_start_compress(&mut started.compress.cinfo, true as boolean);
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

    /// Read-only view of component information
    pub fn components(&self) -> &[CompInfo] {
        self.compress.components()
    }

    fn can_write_more_lines(&self) -> bool {
        self.compress.cinfo.next_scanline < self.compress.cinfo.image_height
    }
}

impl Compress {
    /// Expose components for modification, e.g. to set chroma subsampling
    pub fn components_mut(&mut self) -> &mut [CompInfo] {
        unsafe {
            slice::from_raw_parts_mut(self.cinfo.comp_info, self.cinfo.num_components as usize)
        }
    }

    /// Read-only view of component information
    #[must_use]
    pub fn components(&self) -> &[CompInfo] {
        unsafe { slice::from_raw_parts(self.cinfo.comp_info, self.cinfo.num_components as usize) }
    }
}

impl<W> CompressStarted<W> {
    /// Returns Ok(()) if all lines in `image_src` (not necessarily all lines of the image) were written
    ///
    /// ## Panics
    ///
    /// It may panic, like all functions of this library.
    pub fn write_scanlines(&mut self, image_src: &[u8]) -> io::Result<()> {
        if self.compress.cinfo.raw_data_in != 0 ||
            self.compress.cinfo.input_components <= 0 ||
            self.compress.cinfo.image_width == 0 {
            return Err(io::ErrorKind::InvalidInput.into());
        }

        let byte_width = self.compress.cinfo.image_width as usize * self.compress.cinfo.input_components as usize;
        for rows in image_src.chunks(MAX_MCU_HEIGHT * byte_width) {
            let mut row_pointers = ArrayVec::<_, MAX_MCU_HEIGHT>::new();
            for row in rows.chunks_exact(byte_width) {
                row_pointers.push(row.as_ptr());
            }

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
            panic!("Too many components: declared {}, got {}", num_components, image_src.len());
        }

        for (ci, comp_info) in self.components().iter().enumerate() {
            if comp_info.row_stride() * comp_info.col_stride() > image_src[ci].len() {
                panic!("Bitmap too small. Expected {}x{}, got {}", comp_info.row_stride(), comp_info.col_stride(), image_src[ci].len());
            }
        }

        let mut start_row = self.compress.cinfo.next_scanline as usize;
        while self.can_write_more_lines() {
            unsafe {
                let mut row_ptrs = [[ptr::null::<u8>(); MAX_MCU_HEIGHT]; MAX_COMPONENTS];
                let mut comp_ptrs = [ptr::null::<*const u8>(); MAX_COMPONENTS];

                for (ci, comp_info) in self.components().iter().enumerate() {
                    let row_stride = comp_info.row_stride();

                    let input_height = image_src[ci].len() / row_stride;

                    let comp_start_row = start_row * comp_info.v_samp_factor as usize
                        / self.compress.cinfo.max_v_samp_factor as usize;
                    let comp_height = min(
                        input_height - comp_start_row,
                        DCTSIZE * comp_info.v_samp_factor as usize,
                    );
                    assert!(comp_height >= 8);

                    for ri in 0..comp_height {
                        let start_offset = (comp_start_row + ri) * row_stride;
                        row_ptrs[ci][ri] = image_src[ci][start_offset .. start_offset + row_stride].as_ptr();
                    }
                    for ri in comp_height..mcu_height {
                        row_ptrs[ci][ri] = ptr::null();
                    }
                    comp_ptrs[ci] = row_ptrs[ci].as_ptr();
                }

                let rows_written = ffi::jpeg_write_raw_data(&mut self.compress.cinfo, comp_ptrs.as_ptr(), mcu_height as u32) as usize;
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
    /// See `jpeg_set_colorspace` in libjpeg docs
    pub fn set_color_space(&mut self, color_space: ColorSpace) {
        unsafe {
            ffi::jpeg_set_colorspace(&mut self.cinfo, color_space);
        }
    }

    /// Image size of the input
    pub fn set_size(&mut self, width: usize, height: usize) {
        self.cinfo.image_width = width as JDIMENSION;
        self.cinfo.image_height = height as JDIMENSION;
    }

    /// libjpeg's `input_gamma` = image gamma of input image
    #[deprecated(note = "it doesn't do anything")]
    pub fn set_gamma(&mut self, gamma: f64) {
        self.cinfo.input_gamma = gamma;
    }

    /// If true, it will use MozJPEG's scan optimization. Makes progressive image files smaller.
    pub fn set_optimize_scans(&mut self, opt: bool) {
        unsafe {
            ffi::jpeg_c_set_bool_param(&mut self.cinfo, J_BOOLEAN_PARAM::JBOOLEAN_OPTIMIZE_SCANS, opt as boolean);
        }
        if !opt {
            self.cinfo.scan_info = ptr::null();
        }
    }

    /// If 1-100 (non-zero), it will use MozJPEG's smoothing.
    pub fn set_smoothing_factor(&mut self, smoothing_factor: u8) {
        self.cinfo.smoothing_factor = smoothing_factor as c_int;
    }

    /// Set to `false` to make files larger for no reason
    pub fn set_optimize_coding(&mut self, opt: bool) {
        self.cinfo.optimize_coding = opt as boolean;
    }

    /// Specifies whether multiple scans should be considered during trellis
    /// quantization.
    pub fn set_use_scans_in_trellis(&mut self, opt: bool) {
        unsafe {
            ffi::jpeg_c_set_bool_param(&mut self.cinfo, J_BOOLEAN_PARAM::JBOOLEAN_USE_SCANS_IN_TRELLIS, opt as boolean);
        }
    }

    /// You can only turn it on
    pub fn set_progressive_mode(&mut self) {
        unsafe {
            ffi::jpeg_simple_progression(&mut self.cinfo);
        }
    }

    /// One scan for all components looks best. Other options may flash grayscale or green images.
    pub fn set_scan_optimization_mode(&mut self, mode: ScanMode) {
        unsafe {
            ffi::jpeg_c_set_int_param(&mut self.cinfo, J_INT_PARAM::JINT_DC_SCAN_OPT_MODE, mode as c_int);
            ffi::jpeg_set_defaults(&mut self.cinfo);
        }
    }

    /// Reset to libjpeg v6 settings
    ///
    /// It gives files identical with libjpeg-turbo
    pub fn set_fastest_defaults(&mut self) {
        unsafe {
            ffi::jpeg_c_set_int_param(&mut self.cinfo, J_INT_PARAM::JINT_COMPRESS_PROFILE, ffi::JINT_COMPRESS_PROFILE_VALUE::JCP_FASTEST as c_int);
            ffi::jpeg_set_defaults(&mut self.cinfo);
        }
    }

    /// Advanced. See `raw_data_in` in libjpeg docs.
    pub fn set_raw_data_in(&mut self, opt: bool) {
        self.cinfo.raw_data_in = opt as boolean;
    }

    /// Set image quality. Values 60-80 are recommended.
    pub fn set_quality(&mut self, quality: f32) {
        unsafe {
            ffi::jpeg_set_quality(&mut self.cinfo, quality as c_int, false as boolean);
        }
    }

    /// Instead of quality setting, use a specific quantization table.
    pub fn set_luma_qtable(&mut self, qtable: &QTable) {
        unsafe {
            ffi::jpeg_add_quant_table(&mut self.cinfo, 0, qtable.as_ptr(), 100, 1);
        }
    }

    /// Instead of quality setting, use a specific quantization table for color.
    pub fn set_chroma_qtable(&mut self, qtable: &QTable) {
        unsafe {
            ffi::jpeg_add_quant_table(&mut self.cinfo, 1, qtable.as_ptr(), 100, 1);
        }
    }

    /// Sets chroma subsampling, separately for Cb and Cr channels.
    /// Instead of setting samples per pixel, like in `cinfo`'s `x_samp_factor`,
    /// it sets size of chroma "pixels" per luma pixel.
    ///
    /// * `(1,1), (1,1)` == 4:4:4
    /// * `(2,1), (2,1)` == 4:2:2
    /// * `(2,2), (2,2)` == 4:2:0
    pub fn set_chroma_sampling_pixel_sizes(&mut self, cb: (u8, u8), cr: (u8, u8)) {
        let max_sampling_h = cb.0.max(cr.0);
        let max_sampling_v = cb.1.max(cr.1);

        let px_sizes = [(1, 1), cb, cr];
        for (c, (h, v)) in self.components_mut().iter_mut().zip(px_sizes) {
            c.h_samp_factor = (max_sampling_h / h).into();
            c.v_samp_factor = (max_sampling_v / v).into();
        }
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
    let mut cinfo = Compress::new(ColorSpace::JCS_YCbCr);

    assert_eq!(3, cinfo.components().len());

    cinfo.set_size(17, 33);

    #[allow(deprecated)] {
        cinfo.set_gamma(1.0);
    }

    cinfo.set_progressive_mode();
    cinfo.set_scan_optimization_mode(ScanMode::AllComponentsTogether);

    cinfo.set_raw_data_in(true);

    cinfo.set_quality(88.);

    cinfo.set_chroma_sampling_pixel_sizes((1,1), (1,1));
    for c in cinfo.components().iter() {
        assert_eq!(c.v_samp_factor, 1);
        assert_eq!(c.h_samp_factor, 1);
    }

    cinfo.set_chroma_sampling_pixel_sizes((2,2), (2,2));
    for (c, samp) in cinfo.components().iter().zip([2, 1, 1]) {
        assert_eq!(c.v_samp_factor, samp);
        assert_eq!(c.h_samp_factor, samp);
    }

    let mut cinfo = cinfo.start_compress(Vec::new()).unwrap();

    cinfo.write_marker(Marker::APP(2), "Hello World".as_bytes());

    assert_eq!(24, cinfo.components()[0].row_stride());
    assert_eq!(40, cinfo.components()[0].col_stride());
    assert_eq!(16, cinfo.components()[1].row_stride());
    assert_eq!(24, cinfo.components()[1].col_stride());
    assert_eq!(16, cinfo.components()[2].row_stride());
    assert_eq!(24, cinfo.components()[2].col_stride());

    let bitmaps = cinfo
        .components()
        .iter()
        .map(|c| vec![128u8; c.row_stride() * c.col_stride()])
        .collect::<Vec<_>>();

    assert!(cinfo.write_raw_data(&bitmaps.iter().map(|c| &c[..]).collect::<Vec<_>>()));

    cinfo.finish().unwrap();
}

#[test]
fn convert_colorspace() {
    let mut cinfo = Compress::new(ColorSpace::JCS_RGB);
    cinfo.set_color_space(ColorSpace::JCS_GRAYSCALE);
    assert_eq!(1, cinfo.components().len());

    cinfo.set_size(33, 15);
    cinfo.set_quality(44.);

    let mut cinfo = cinfo.start_compress(Vec::new()).unwrap();

    let scanlines = vec![127u8; 33*15*3];
    cinfo.write_scanlines(&scanlines).unwrap();

    let res = cinfo.finish().unwrap();
    assert!(!res.is_empty());
}
