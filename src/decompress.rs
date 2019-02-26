//! You don't need to use this module directly.
//!
//! See `mozjpeg::Decompress` struct instead.
use crate::ffi;
use crate::ffi::jpeg_decompress_struct;
use crate::ffi::DCTSIZE;
use crate::ffi::JPEG_LIB_VERSION;
use crate::ffi::J_COLOR_SPACE as COLOR_SPACE;
use std::os::raw::{c_int, c_uchar, c_ulong, c_void};
use crate::colorspace::ColorSpace;
use crate::colorspace::ColorSpaceExt;
use crate::component::CompInfo;
use crate::component::CompInfoExt;
use crate::errormgr::ErrorMgr;
use crate::errormgr::PanicingErrorMgr;
use crate::marker::Marker;
use crate::vec::VecUninitExtender;
use libc::fdopen;
use std::cmp::min;
use std::fs::File;
use std::io;
use std::marker::PhantomData;
use std::mem;
#[cfg(unix)]
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::ptr;
use std::slice;

const MAX_MCU_HEIGHT: usize = 16;
const MAX_COMPONENTS: usize = 4;

/// Empty list of markers
///
/// By default markers are not read from JPEG files.
pub const NO_MARKERS: &[Marker] = &[];

/// App 0-14 and comment markers
///
/// ```rust,ignore
/// Decompress::with_markers(ALL_MARKERS)
/// ```
pub const ALL_MARKERS: &[Marker] = &[
    Marker::APP(0), Marker::APP(1), Marker::APP(2), Marker::APP(3), Marker::APP(4),
    Marker::APP(5), Marker::APP(6), Marker::APP(7), Marker::APP(8), Marker::APP(9),
    Marker::APP(10), Marker::APP(11), Marker::APP(12), Marker::APP(13), Marker::APP(14),
    Marker::COM,
];

/// Algorithm for the DCT step.
#[derive(Clone, Copy, Debug)]
pub enum DctMethod {
    /// slow but accurate integer algorithm
    IntegerSlow,
    /// faster, less accurate integer method
    IntegerFast,
    /// floating-point method
    Float,
}

/// Use `Decompress` static methods instead of creating this directly
pub struct DecompressConfig<'markers> {
    save_markers: &'markers [Marker],
    err: Option<ErrorMgr>,
}

impl<'markers> DecompressConfig<'markers> {
    #[inline]
    pub fn new() -> Self {
        DecompressConfig {
            err: None,
            save_markers: NO_MARKERS,
        }
    }

    #[inline]
    fn create<'a>(self) -> Decompress<'a> {
        let mut d = Decompress::new_err(self.err.unwrap_or_else(<ErrorMgr as PanicingErrorMgr>::new));
        for &marker in self.save_markers {
            d.save_marker(marker);
        }
        d
    }

    #[inline]
    pub fn with_err(mut self, err: ErrorMgr) -> Self {
        self.err = Some(err);
        self
    }

    #[inline]
    pub fn with_markers(mut self, save_markers: &'markers [Marker]) -> Self {
        self.save_markers = save_markers;
        self
    }

    #[inline]
    #[cfg(unix)]
    pub fn from_path<P: AsRef<Path>>(self, path: P) -> io::Result<Decompress<'static>> {
        self.from_file(File::open(path)?)
    }

    #[inline]
    #[cfg(unix)]
    pub fn from_file(self, file: File) -> io::Result<Decompress<'static>> {
        let mut d = self.create();
        d.set_file_src(Box::new(file))?;
        d.read_header()?;
        Ok(d)
    }

    #[inline]
    pub fn from_mem<'src>(self, mem: &'src [u8]) -> io::Result<Decompress<'src>> {
        let mut d = self.create();
        d.set_mem_src(mem);
        d.read_header()?;
        Ok(d)
    }
}

/// Get pixels out of a JPEG file
///
/// High-level wrapper for `jpeg_decompress_struct`
///
/// ```rust,ignore
/// let d = Decompress::new_path("image.jpg");
/// ```
pub struct Decompress<'src> {
    cinfo: jpeg_decompress_struct,
    own_error: Box<ErrorMgr>,
    own_file: Option<Box<File>>,
    // Informs the borrow checker that the memory given in src must outlive the `jpeg_decompress_struct`
    _mem_marker: PhantomData<&'src [u8]>,
}

/// Marker type and data slice returned by `MarkerIter`
pub struct MarkerData<'a> {
    pub marker: Marker,
    pub data: &'a [u8],
}

/// See `Decompress.markers()`
pub struct MarkerIter<'a> {
    marker_list: *mut ffi::jpeg_marker_struct,
    _uhh: ::std::marker::PhantomData<MarkerData<'a>>,
}

impl<'a> Iterator for MarkerIter<'a> {
    type Item = MarkerData<'a>;
    fn next(&mut self) -> Option<MarkerData<'a>> {
        if self.marker_list.is_null() {
            return None;
        }
        unsafe {
            let last = &*self.marker_list;
            self.marker_list = last.next;
            Some(MarkerData {
                marker: last.marker.into(),
                data: ::std::slice::from_raw_parts(last.data, last.data_length as usize),
            })
        }
    }
}

impl<'src> Decompress<'src> {
    #[inline]
    pub fn with_err(err: ErrorMgr) -> DecompressConfig<'static> {
        Self::config().with_err(err)
    }

    #[inline]
    pub fn with_markers(save_markers: &[Marker]) -> DecompressConfig<'_> {
        Self::config().with_markers(save_markers)
    }

    #[inline]
    #[cfg(unix)]
    /// Decode file at path
    pub fn new_path<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        Self::config().from_path(path)
    }

    /// Decode an already-opened file
    #[inline]
    #[cfg(unix)]
    pub fn new_file(file: File) -> io::Result<Self> {
        Self::config().from_file(file)
    }

    #[inline]
    pub fn new_mem(mem: &'src [u8]) -> io::Result<Self> {
        Self::config().from_mem(mem)
    }

    #[inline]
    fn config() -> DecompressConfig<'static> {
        DecompressConfig::new()
    }

    fn new_err(err: ErrorMgr) -> Self {
        unsafe {
            let mut newself = Decompress {
                cinfo: mem::zeroed(),
                own_error: Box::new(err),
                own_file: None,
                _mem_marker: PhantomData,
            };
            newself.cinfo.common.err = &mut *newself.own_error;

            let s = mem::size_of_val(&newself.cinfo);
            ffi::jpeg_CreateDecompress(&mut newself.cinfo, JPEG_LIB_VERSION, s);

            newself
        }
    }

    pub fn components(&self) -> &[CompInfo] {
        unsafe { slice::from_raw_parts(self.cinfo.comp_info, self.cinfo.num_components as usize) }
    }

    pub fn components_mut(&mut self) -> &mut [CompInfo] {
        unsafe {
            slice::from_raw_parts_mut(self.cinfo.comp_info, self.cinfo.num_components as usize)
        }
    }

    #[cfg(unix)]
    fn set_file_src(&mut self, file: Box<File>) -> io::Result<()> {
        unsafe {
            let fh = fdopen(file.as_raw_fd(), b"rb".as_ptr() as *const _);
            if fh.is_null() {
                return Err(io::Error::last_os_error());
            }
            ffi::jpeg_stdio_src(&mut self.cinfo, fh)
        }
        self.own_file = Some(file);
        Ok(())
    }

    fn set_mem_src(&mut self, file: &'src [u8]) {
        unsafe {
            ffi::jpeg_mem_src(&mut self.cinfo, file.as_ptr(), file.len() as c_ulong);
        }
    }

    /// Result here is mostly useless, because it will panic if the file is invalid
    fn read_header(&mut self) -> io::Result<()> {
        let res = unsafe { ffi::jpeg_read_header(&mut self.cinfo, 0) };
        if res == 1 {
            Ok(())
        } else {
            Err(io::Error::new(io::ErrorKind::Other, format!("JPEG err {}", res)))
        }
    }

    pub fn color_space(&self) -> COLOR_SPACE {
        self.cinfo.jpeg_color_space
    }

    pub fn gamma(&self) -> f64 {
        self.cinfo.output_gamma
    }

    /// Markers are available only if you enable them via `with_markers()`
    pub fn markers(&self) -> MarkerIter<'_> {
        MarkerIter {
            marker_list: self.cinfo.marker_list,
            _uhh: PhantomData,
        }
    }

    fn save_marker(&mut self, marker: Marker) {
        unsafe {
            ffi::jpeg_save_markers(&mut self.cinfo, marker.into(), 0xFFFF);
        }
    }

    /// width,height
    #[inline]
    pub fn size(&self) -> (usize, usize) {
        (self.width(), self.height())
    }

    #[inline]
    pub fn width(&self) -> usize {
        self.cinfo.image_width as usize
    }

    #[inline]
    pub fn height(&self) -> usize {
        self.cinfo.image_height as usize
    }

    fn set_raw_data_out(&mut self, raw: bool) {
        self.cinfo.raw_data_out = raw as ffi::boolean;
    }

    /// Start decompression with conversion to RGB
    pub fn rgb(mut self) -> io::Result<DecompressStarted<'src>> {
        self.cinfo.out_color_space = ffi::J_COLOR_SPACE::JCS_RGB;
        DecompressStarted::start_decompress(self)
    }

    /// Start decompression with conversion to RGBA
    pub fn rgba(mut self) -> io::Result<DecompressStarted<'src>> {
        self.cinfo.out_color_space = ffi::J_COLOR_SPACE::JCS_EXT_RGBA;
        DecompressStarted::start_decompress(self)
    }

    /// Start decompression with conversion to grayscale.
    pub fn grayscale(mut self) -> io::Result<DecompressStarted<'src>> {
        self.cinfo.out_color_space = ffi::J_COLOR_SPACE::JCS_GRAYSCALE;
        DecompressStarted::start_decompress(self)
    }

    /// Selects the algorithm used for the DCT step.
    pub fn dct_method(&mut self, method: DctMethod) {
        self.cinfo.dct_method = match method {
            DctMethod::IntegerSlow => ffi::J_DCT_METHOD::JDCT_ISLOW,
            DctMethod::IntegerFast => ffi::J_DCT_METHOD::JDCT_IFAST,
            DctMethod::Float => ffi::J_DCT_METHOD::JDCT_FLOAT,
        }
    }

    // If `true`, do careful upsampling of chroma components.  If `false`,
    // a faster but sloppier method is used.  Default is `true`.  The visual
    // impact of the sloppier method is often very small.
    pub fn do_fancy_upsampling(&mut self, value: bool) {
        self.cinfo.do_fancy_upsampling = value as ffi::boolean;
    }

    /// If `true`, interblock smoothing is applied in early stages of decoding
    /// progressive JPEG files; if `false`, not.  Default is `true`.  Early
    /// progression stages look "fuzzy" with smoothing, "blocky" without.
    /// In any case, block smoothing ceases to be applied after the first few
    /// AC coefficients are known to full accuracy, so it is relevant only
    /// when using buffered-image mode for progressive images.
    pub fn do_block_smoothing(&mut self, value: bool) {
        self.cinfo.do_block_smoothing = value as ffi::boolean;
    }

    pub fn raw(mut self) -> io::Result<DecompressStarted<'src>> {
        self.set_raw_data_out(true);
        DecompressStarted::start_decompress(self)
    }

    fn out_color_space(&self) -> ColorSpace {
        self.cinfo.out_color_space
    }

    /// Start decompression without colorspace conversion
    pub fn image(self) -> io::Result<Format<'src>> {
        use crate::ffi::J_COLOR_SPACE::*;
        match self.out_color_space() {
            JCS_RGB => Ok(Format::RGB(DecompressStarted::start_decompress(self)?)),
            JCS_CMYK => Ok(Format::CMYK(DecompressStarted::start_decompress(self)?)),
            JCS_GRAYSCALE => Ok(Format::Gray(DecompressStarted::start_decompress(self)?)),
            format => Err(io::Error::new(io::ErrorKind::Other, format!("{:?}", format))),
        }
    }

    /// Rescales the output image by `numerator / 8` during decompression.
    /// `numerator` must be between 1 and 16.
    /// Thus setting a value of `8` will result in an unscaled image.
    pub fn scale(&mut self, numerator: u8) {
        assert!(1 <= numerator && numerator <= 16, "numerator must be between 1 and 16");
        self.cinfo.scale_num = numerator.into();
        self.cinfo.scale_denom = 8;
    }
}

/// See `Decompress.image()`
pub enum Format<'a> {
    RGB(DecompressStarted<'a>),
    Gray(DecompressStarted<'a>),
    CMYK(DecompressStarted<'a>),
}

/// See methods on `Decompress`
pub struct DecompressStarted<'src> {
    dec: Decompress<'src>,
}

impl<'src> DecompressStarted<'src> {
    fn start_decompress(mut dec: Decompress<'src>) -> io::Result<Self> {
        let res = unsafe { ffi::jpeg_start_decompress(&mut dec.cinfo) };
        if 0 != res {
            Ok(DecompressStarted { dec })
        } else {
            Err(io::Error::new(io::ErrorKind::Other, format!("JPEG err {}", res)))
        }
    }

    pub fn color_space(&self) -> ColorSpace {
        self.dec.out_color_space()
    }

    fn read_more_chunks(&self) -> bool {
        self.dec.cinfo.output_scanline < self.dec.cinfo.output_height
    }

    pub fn read_raw_data(&mut self, image_dest: &mut [&mut Vec<u8>]) {
        while self.read_more_chunks() {
            self.read_raw_data_chunk(image_dest);
        }
    }

    fn read_raw_data_chunk(&mut self, image_dest: &mut [&mut Vec<u8>]) {
        assert!(0 != self.dec.cinfo.raw_data_out, "Raw data not set");

        let mcu_height = self.dec.cinfo.max_v_samp_factor as usize * DCTSIZE;
        if mcu_height > MAX_MCU_HEIGHT {
            panic!("Subsampling factor too large");
        }

        let num_components = self.dec.components().len();
        if num_components > MAX_COMPONENTS || num_components > image_dest.len() {
            panic!("Too many components. Image has {}, destination vector has {} (max supported is {})", num_components, image_dest.len(), MAX_COMPONENTS);
        }

        unsafe {
            let mut row_ptrs = [[ptr::null_mut::<u8>(); MAX_MCU_HEIGHT]; MAX_COMPONENTS];
            let mut comp_ptrs = [ptr::null_mut::<*mut u8>(); MAX_COMPONENTS];
            for (ci, comp_info) in self.dec.components().iter().enumerate() {
                let row_stride = comp_info.row_stride();

                let comp_height = comp_info.v_samp_factor as usize * DCTSIZE;
                let original_len = image_dest[ci].len();
                image_dest[ci].extend_uninit(comp_height * row_stride);
                for ri in 0..comp_height {
                    let start = original_len + ri * row_stride;
                    row_ptrs[ci][ri] = (&mut image_dest[ci][start.. start + row_stride]).as_mut_ptr();
                }
                for ri in comp_height..mcu_height {
                    row_ptrs[ci][ri] = ptr::null_mut();
                }
                comp_ptrs[ci] = row_ptrs[ci].as_mut_ptr();
            }

            let lines_read = ffi::jpeg_read_raw_data(&mut self.dec.cinfo, comp_ptrs.as_mut_ptr(), mcu_height as u32) as usize;

            assert_eq!(lines_read, mcu_height); // Partial reads would make subsampled height tricky to define
        }
    }

    pub fn width(&self) -> usize {
        self.dec.cinfo.output_width as usize
    }

    pub fn height(&self) -> usize {
        self.dec.cinfo.output_height as usize
    }

    pub fn read_scanlines<T: Copy>(&mut self) -> Option<Vec<T>> {
        let num_components = self.color_space().num_components();
        assert_eq!(num_components, mem::size_of::<T>());
        let width = self.width();
        let height = self.height();
        let mut image_dst: Vec<T> = Vec::with_capacity(self.height() * width);
        unsafe {
            image_dst.extend_uninit(height * width);

            while self.read_more_chunks() {
                let start_line = self.dec.cinfo.output_scanline as usize;
                let rest: &mut [T] = &mut image_dst[width * start_line..];
                let rows = (&mut rest.as_mut_ptr()) as *mut *mut T;

                let rows_read = ffi::jpeg_read_scanlines(&mut self.dec.cinfo, rows as *mut *mut u8, 1) as usize;
                debug_assert_eq!(start_line + rows_read, self.dec.cinfo.output_scanline as usize, "wat {}/{} at {}", rows_read, height, start_line);
                if 0 == rows_read {
                    return None;
                }
            }
        }
        Some(image_dst)
    }

    pub fn components(&self) -> &[CompInfo] {
        self.dec.components()
    }

    pub fn components_mut(&mut self) -> &[CompInfo] {
        self.dec.components_mut()
    }

    pub fn finish_decompress(mut self) -> bool {
        unsafe { 0 != ffi::jpeg_finish_decompress(&mut self.dec.cinfo) }
    }
}

impl<'src> Drop for Decompress<'src> {
    fn drop(&mut self) {
        unsafe {
            ffi::jpeg_destroy_decompress(&mut self.cinfo);
        }
    }
}

#[test]
fn read_file() {
    use crate::colorspace::ColorSpace;
    use crate::colorspace::ColorSpaceExt;
    use std::fs::File;
    use std::io::Read;

    let mut data = Vec::new();
    File::open("tests/test.jpg").unwrap().read_to_end(&mut data).unwrap();
    assert_eq!(2169, data.len());

    let dinfo = Decompress::new_mem(&data[..]).unwrap();

    assert_eq!(1.0, dinfo.gamma());
    assert_eq!(ColorSpace::JCS_YCbCr, dinfo.color_space());
    assert_eq!(dinfo.components().len(), dinfo.color_space().num_components() as usize);


    assert_eq!((45, 30), dinfo.size());
    {
        let comps = dinfo.components();
        assert_eq!(2, comps[0].h_samp_factor);
        assert_eq!(2, comps[0].v_samp_factor);

        assert_eq!(48, comps[0].row_stride());
        assert_eq!(32, comps[0].col_stride());

        assert_eq!(1, comps[1].h_samp_factor);
        assert_eq!(1, comps[1].v_samp_factor);
        assert_eq!(1, comps[2].h_samp_factor);
        assert_eq!(1, comps[2].v_samp_factor);

        assert_eq!(24, comps[1].row_stride());
        assert_eq!(16, comps[1].col_stride());
        assert_eq!(24, comps[2].row_stride());
        assert_eq!(16, comps[2].col_stride());
    }

    let mut dinfo = dinfo.raw().unwrap();

    let mut has_chunks = false;
    let mut bitmaps = [&mut Vec::new(), &mut Vec::new(), &mut Vec::new()];
    while dinfo.read_more_chunks() {
        has_chunks = true;
        dinfo.read_raw_data_chunk(&mut bitmaps);
        assert_eq!(bitmaps[0].len(), 4 * bitmaps[1].len());
    }
    assert!(has_chunks);

    for (bitmap, comp) in bitmaps.iter().zip(dinfo.components()) {
        assert_eq!(comp.row_stride() * comp.col_stride(), bitmap.len());
    }

    assert!(dinfo.finish_decompress());
}

#[test]
#[cfg(unix)]
fn no_markers() {
    use crate::colorspace::ColorSpace;
    use crate::colorspace::ColorSpaceExt;
    use std::fs::File;
    use std::io::Read;

    let dinfo = Decompress::new_path("tests/test.jpg").unwrap();
    assert_eq!(0, dinfo.markers().count());

    let dinfo = Decompress::with_markers(&[]).from_path("tests/test.jpg").unwrap();
    assert_eq!(0, dinfo.markers().count());
}

#[test]
fn read_file_rgb() {
    use crate::colorspace::ColorSpace;
    use crate::colorspace::ColorSpaceExt;
    use std::fs::File;
    use std::io::Read;

    let mut data = Vec::new();
    File::open("tests/test.jpg").unwrap().read_to_end(&mut data).unwrap();
    let dinfo = Decompress::with_markers(ALL_MARKERS).from_mem(&data[..]).unwrap();

    assert_eq!(ColorSpace::JCS_YCbCr, dinfo.color_space());

    assert_eq!(1, dinfo.markers().count());

    let mut dinfo = dinfo.rgb().unwrap();
    assert_eq!(ColorSpace::JCS_RGB, dinfo.color_space());
    assert_eq!(dinfo.components().len(), dinfo.color_space().num_components() as usize);

    let bitmap: Vec<(u8, u8, u8)> = dinfo.read_scanlines().unwrap();
    assert_eq!(bitmap.len(), 45 * 30);

    assert!(!bitmap.contains(&(0, 0, 0)));

    assert!(dinfo.finish_decompress());
}
