//! See the `Decompress` struct instead. You don't need to use this module directly.
use std::io::BufRead;
use std::io::BufReader;
use crate::readsrc::SourceMgr;
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
use crate::errormgr::unwinding_error_mgr;
use crate::marker::Marker;
use crate::vec::VecUninitExtender;
use libc::fdopen;
use std::cmp::min;
use std::fs::File;
use std::io;
use std::marker::PhantomData;
use std::mem;
use std::path::Path;
use std::ptr;
use std::ptr::addr_of_mut;
use std::slice;

const MAX_MCU_HEIGHT: usize = 16;
const MAX_COMPONENTS: usize = 4;

/// Empty list of markers
///
/// By default markers are not read from JPEG files.
pub const NO_MARKERS: &[Marker] = &[];

/// App 0-14 and comment markers
///
/// ```rust
/// # use mozjpeg::*;
/// Decompress::with_markers(ALL_MARKERS);
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
pub struct DecompressBuilder<'markers> {
    save_markers: &'markers [Marker],
    err_mgr: Option<Box<ErrorMgr>>,
}

#[deprecated(note = "Renamed to DecompressBuilder")]
#[doc(hidden)]
pub use DecompressBuilder as DecompressConfig;

impl<'markers> DecompressBuilder<'markers> {
    #[inline]
    pub const fn new() -> Self {
        DecompressBuilder {
            err_mgr: None,
            save_markers: NO_MARKERS,
        }
    }

    #[inline]
    pub fn with_err(mut self, err: ErrorMgr) -> Self {
        self.err_mgr = Some(Box::new(err));
        self
    }

    #[inline]
    pub const fn with_markers(mut self, save_markers: &'markers [Marker]) -> Self {
        self.save_markers = save_markers;
        self
    }

    #[inline]
    pub fn from_path<P: AsRef<Path>>(self, path: P) -> io::Result<Decompress<BufReader<File>>> {
        self.from_file(File::open(path.as_ref())?)
    }

    /// Reads from an already-open `File`.
    /// Use `from_reader` if you want to customize buffer size.
    #[inline]
    pub fn from_file(self, file: File) -> io::Result<Decompress<BufReader<File>>> {
        self.from_reader(BufReader::new(file))
    }

    /// Reads from a `Vec` or a slice.
    #[inline]
    pub fn from_mem(self, mem: &[u8]) -> io::Result<Decompress<&[u8]>> {
        self.from_reader(mem)
    }

    /// Takes `BufReader`. If you have `io::Read`, wrap it in `io::BufReader::new(read)`.
    #[inline]
    pub fn from_reader<R: BufRead>(self, reader: R) -> io::Result<Decompress<R>> {
        Decompress::from_builder_and_reader(self, reader)
    }
}

/// Get pixels out of a JPEG file
///
/// High-level wrapper for `jpeg_decompress_struct`
///
/// ```rust
/// # use mozjpeg::*;
/// # fn t() -> std::io::Result<()> {
/// let d = Decompress::new_path("image.jpg")?;
/// # Ok(()) }
/// ```
pub struct Decompress<R> {
    cinfo: jpeg_decompress_struct,
    err_mgr: Box<ErrorMgr>,
    src_mgr: Box<SourceMgr<R>>,
}

/// Marker type and data slice returned by `MarkerIter`
pub struct MarkerData<'a> {
    pub marker: Marker,
    pub data: &'a [u8],
}

/// See `Decompress.markers()`
pub struct MarkerIter<'a> {
    marker_list: *mut ffi::jpeg_marker_struct,
    _references: ::std::marker::PhantomData<MarkerData<'a>>,
}

impl<'a> Iterator for MarkerIter<'a> {
    type Item = MarkerData<'a>;
    #[inline]
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

impl Decompress<()> {
    /// Short for builder().with_err()
    #[inline]
    #[doc(hidden)]
    pub fn with_err(err_mgr: ErrorMgr) -> DecompressBuilder<'static> {
        DecompressBuilder::new().with_err(err_mgr)
    }

    /// Short for builder().with_markers()
    #[inline]
    #[doc(hidden)]
    pub fn with_markers(markers: &[Marker]) -> DecompressBuilder<'_> {
        DecompressBuilder::new().with_markers(markers)
    }

    /// Use builder()
    #[deprecated(note = "renamed to builder()")]
    #[doc(hidden)]
    pub fn config() -> DecompressBuilder<'static> {
        DecompressBuilder::new()
    }

    /// This is `DecompressBuilder::new()`
    #[inline]
    pub const fn builder() -> DecompressBuilder<'static> {
        DecompressBuilder::new()
    }
}

impl Decompress<BufReader<File>> {
    /// Decode file at path
    #[inline]
    pub fn new_path<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        DecompressBuilder::new().from_path(path)
    }

    /// Decode an already-opened file
    #[inline]
    pub fn new_file(file: File) -> io::Result<Self> {
        DecompressBuilder::new().from_file(file)
    }
}

impl<'mem> Decompress<&'mem [u8]> {
    /// Decode from a JPEG file already in memory
    #[inline]
    pub fn new_mem(mem: &'mem [u8]) -> io::Result<Self> {
        DecompressBuilder::new().from_mem(mem)
    }
}

impl<R> Decompress<R> {
    /// Decode from an `io::BufRead`, which is `BufReader` wrapping any `io::Read`.
    #[inline]
    pub fn new_reader(reader: R) -> io::Result<Self> where R: BufRead {
        DecompressBuilder::new().from_reader(reader)
    }

    fn from_builder_and_reader(builder: DecompressBuilder<'_>, reader: R) -> io::Result<Self> where R: BufRead {
        let src_mgr = Box::new(SourceMgr::new(reader)?);
        let err_mgr = builder.err_mgr.unwrap_or_else(unwinding_error_mgr);
        unsafe {
            let mut newself = Decompress {
                cinfo: mem::zeroed(),
                src_mgr,
                err_mgr,
            };
            newself.cinfo.common.err = addr_of_mut!(*newself.err_mgr);
            ffi::jpeg_create_decompress(&mut newself.cinfo);
            newself.cinfo.src = addr_of_mut!(newself.src_mgr.iface);
            for &marker in builder.save_markers {
                newself.save_marker(marker);
            }
            newself.read_header()?;
            Ok(newself)
        }
    }

    #[inline]
    pub fn components(&self) -> &[CompInfo] {
        unsafe {
            slice::from_raw_parts(self.cinfo.comp_info, self.cinfo.num_components as usize)
        }
    }

    #[inline]
    pub(crate) fn components_mut(&mut self) -> &mut [CompInfo] {
        unsafe {
            slice::from_raw_parts_mut(self.cinfo.comp_info, self.cinfo.num_components as usize)
        }
    }

    /// Result here is mostly useless, because it will panic if the file is invalid
    #[inline]
    fn read_header(&mut self) -> io::Result<()> {
        let res = unsafe { ffi::jpeg_read_header(&mut self.cinfo, 0) };
        if res == 1 || res == 2 {
            Ok(())
        } else {
            Err(io::Error::new(io::ErrorKind::Other, format!("JPEG err {}", res)))
        }
    }

    #[inline]
    pub fn color_space(&self) -> COLOR_SPACE {
        self.cinfo.jpeg_color_space
    }

    /// It's generally bogus in libjpeg
    #[inline]
    pub fn gamma(&self) -> f64 {
        self.cinfo.output_gamma
    }

    /// Markers are available only if you enable them via `with_markers()`
    #[inline]
    pub fn markers(&self) -> MarkerIter<'_> {
        MarkerIter {
            marker_list: self.cinfo.marker_list,
            _references: PhantomData,
        }
    }

    #[inline]
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

    /// Start decompression with conversion to RGB
    #[inline(always)]
    pub fn rgb(self) -> io::Result<DecompressStarted<R>> {
        self.to_colorspace(ffi::J_COLOR_SPACE::JCS_RGB)
    }

    /// Start decompression with conversion to `colorspace`
    pub fn to_colorspace(mut self, colorspace: ColorSpace) -> io::Result<DecompressStarted<R>> {
        self.cinfo.out_color_space = colorspace;
        DecompressStarted::start_decompress(self)
    }

    /// Start decompression with conversion to RGBA
    #[inline(always)]
    pub fn rgba(self) -> io::Result<DecompressStarted<R>> {
        self.to_colorspace(ffi::J_COLOR_SPACE::JCS_EXT_RGBA)
    }

    /// Start decompression with conversion to grayscale.
    #[inline(always)]
    pub fn grayscale(self) -> io::Result<DecompressStarted<R>> {
        self.to_colorspace(ffi::J_COLOR_SPACE::JCS_GRAYSCALE)
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

    #[inline(always)]
    pub fn raw(mut self) -> io::Result<DecompressStarted<R>> {
        self.cinfo.raw_data_out = true as ffi::boolean;
        DecompressStarted::start_decompress(self)
    }

    fn out_color_space(&self) -> ColorSpace {
        self.cinfo.out_color_space
    }

    /// Start decompression without colorspace conversion
    pub fn image(self) -> io::Result<Format<R>> {
        use crate::ffi::J_COLOR_SPACE::*;
        match self.out_color_space() {
            JCS_RGB => Ok(Format::RGB(DecompressStarted::start_decompress(self)?)),
            JCS_CMYK => Ok(Format::CMYK(DecompressStarted::start_decompress(self)?)),
            JCS_GRAYSCALE => Ok(Format::Gray(DecompressStarted::start_decompress(self)?)),
            _ => Ok(Format::RGB(self.rgb()?))
        }
    }

    /// Rescales the output image by `numerator / 8` during decompression.
    /// `numerator` must be between 1 and 16.
    /// Thus setting a value of `8` will result in an unscaled image.
    #[track_caller]
    #[inline]
    pub fn scale(&mut self, numerator: u8) {
        assert!(1 <= numerator && numerator <= 16, "numerator must be between 1 and 16");
        self.cinfo.scale_num = numerator.into();
        self.cinfo.scale_denom = 8;
    }
}

/// See `Decompress.image()`
pub enum Format<R> {
    RGB(DecompressStarted<R>),
    Gray(DecompressStarted<R>),
    CMYK(DecompressStarted<R>),
}

/// See methods on `Decompress`
pub struct DecompressStarted<R> {
    dec: Decompress<R>,
}

impl<R> DecompressStarted<R> {
    fn start_decompress(dec: Decompress<R>) -> io::Result<Self> {
        let mut dec = DecompressStarted { dec };
        if 0 != unsafe { ffi::jpeg_start_decompress(&mut dec.dec.cinfo) } {
            Ok(dec)
        } else {
            io_suspend_err()
        }
    }

    pub fn color_space(&self) -> ColorSpace {
        self.dec.out_color_space()
    }

    /// Gets the minimal buffer size for using `DecompressStarted::read_scanlines_flat_into`
    #[inline(always)]
    pub fn min_flat_buffer_size(&self) -> usize {
        self.color_space().num_components() * self.width() * self.height()
    }

    fn can_read_more_scanlines(&self) -> bool {
        self.dec.cinfo.output_scanline < self.dec.cinfo.output_height
    }

    #[track_caller]
    pub fn read_raw_data(&mut self, image_dest: &mut [&mut Vec<u8>]) {
        while self.can_read_more_scanlines() {
            self.read_raw_data_chunk(image_dest);
        }
    }

    #[track_caller]
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

    /// Supports any pixel type that is marked as "plain old data", see bytemuck crate.
    ///
    /// Pixels can either have number of bytes matching number of channels, e.g. RGB as
    /// `[u8; 3]` or `rgb::RGB8`, or be an amorphous blob of `u8`s.
    pub fn read_scanlines<T: rgb::Pod>(&mut self) -> io::Result<Vec<T>> {
        let num_components = self.color_space().num_components();
        if num_components != mem::size_of::<T>() && mem::size_of::<T>() != 1 {
            return Err(io::Error::new(io::ErrorKind::Unsupported, format!("pixel size must have {num_components} bytes, but has {}", mem::size_of::<T>())));
        }
        let width = self.width();
        let height = self.height();
        let mut image_dst: Vec<T> = Vec::new();
        let required_len = height * width * (num_components / mem::size_of::<T>());
        image_dst.try_reserve_exact(required_len).map_err(|_| io::ErrorKind::OutOfMemory)?;
        unsafe { image_dst.extend_uninit(required_len); }
        self.read_scanlines_into(&mut image_dst)?;
        Ok(image_dst)
    }

    /// Supports any pixel type that is marked as "plain old data", see bytemuck crate.
    /// `[u8; 3]` and `rgb::RGB8` are fine, for example. `[u8]` is allowed for any pixel type.
    ///
    /// Allocation-less version of `read_scanlines`
    pub fn read_scanlines_into<'dest, T: rgb::Pod>(&mut self, dest: &'dest mut [T]) -> io::Result<&'dest mut [T]> {
        let num_components = self.color_space().num_components();
        let item_size = if mem::size_of::<T>() == 1 {
            num_components
        } else if num_components == mem::size_of::<T>() {
            1
        } else {
            return Err(io::Error::new(io::ErrorKind::Unsupported, format!("pixel size must have {num_components} bytes, but has {}", mem::size_of::<T>())));
        };
        let width = self.width();
        let height = self.height();
        let line_width = width * item_size;
        if dest.len() % line_width != 0 {
            return Err(io::Error::new(io::ErrorKind::Unsupported, format!("destination slice length must be multiple of {width}x{num_components} bytes long, got {}B", dest.len() * mem::size_of::<T>())));
        }
        for row in dest.chunks_exact_mut(line_width) {
            if !self.can_read_more_scanlines() {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            let start_line = self.dec.cinfo.output_scanline as usize;
            let rows = (&mut row.as_mut_ptr()) as *mut *mut T;
            unsafe {
                let rows_read = ffi::jpeg_read_scanlines(&mut self.dec.cinfo, rows as *mut *mut u8, 1) as usize;
                debug_assert_eq!(start_line + rows_read, self.dec.cinfo.output_scanline as usize, "{start_line}+{rows_read} != {} of {height}", self.dec.cinfo.output_scanline);
                if 0 == rows_read {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
            }
        }
        Ok(dest)
    }

    #[deprecated(note = "use read_scanlines::<u8>")]
    #[doc(hidden)]
    pub fn read_scanlines_flat(&mut self) -> io::Result<Vec<u8>> {
        self.read_scanlines()
    }

    #[deprecated(note = "use read_scanlines_into::<u8>")]
    #[doc(hidden)]
    pub fn read_scanlines_flat_into<'dest>(&mut self, dest: &'dest mut [u8]) -> io::Result<&'dest mut [u8]> {
        self.read_scanlines_into(dest)
    }

    pub fn components(&self) -> &[CompInfo] {
        self.dec.components()
    }

    #[deprecated(note = "too late to mutate, use components()")]
    #[doc(hidden)]
    pub fn components_mut(&mut self) -> &[CompInfo] {
        self.dec.components_mut()
    }

    #[deprecated(note = "use finish()")]
    #[doc(hidden)]
    pub fn finish_decompress(self) -> bool {
        self.finish().is_ok()
    }

    pub fn finish(mut self) -> io::Result<()> {
        if 0 != unsafe { ffi::jpeg_finish_decompress(&mut self.dec.cinfo) } {
            Ok(())
        } else {
            io_suspend_err()
        }
    }
}

#[cold]
fn io_suspend_err<T>() -> io::Result<T> {
    Err(io::ErrorKind::WouldBlock.into())
}

impl<R> Drop for Decompress<R> {
    fn drop(&mut self) {
        unsafe {
            ffi::jpeg_destroy_decompress(&mut self.cinfo);
        }
    }
}

#[test]
fn read_incomplete_file() {
    use crate::colorspace::ColorSpace;
    use crate::colorspace::ColorSpaceExt;
    use std::fs::File;
    use std::io::Read;

    let data = std::fs::read("tests/test.jpg").unwrap();
    assert_eq!(2169, data.len());

    let dinfo = Decompress::new_mem(&data[..data.len()/2]).unwrap();
    let mut dinfo = dinfo.rgb().unwrap();
    let _bitmap: Vec<[u8; 3]> = dinfo.read_scanlines().unwrap();
}

#[test]
fn read_file() {
    use crate::colorspace::ColorSpace;
    use crate::colorspace::ColorSpaceExt;
    use std::fs::File;
    use std::io::Read;

    let data = std::fs::read("tests/test.jpg").unwrap();
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
    while dinfo.can_read_more_scanlines() {
        has_chunks = true;
        dinfo.read_raw_data_chunk(&mut bitmaps);
        assert_eq!(bitmaps[0].len(), 4 * bitmaps[1].len());
    }
    assert!(has_chunks);

    for (bitmap, comp) in bitmaps.iter().zip(dinfo.components()) {
        assert_eq!(comp.row_stride() * comp.col_stride(), bitmap.len());
    }

    assert!(dinfo.finish().is_ok());
}

#[test]
fn no_markers() {
    use crate::colorspace::ColorSpace;
    use crate::colorspace::ColorSpaceExt;
    use std::fs::File;
    use std::io::Read;

    // btw tests src manager with 1-byte len, which requires libjpeg to refill the buffer a lot
    let tricky_buf = io::BufReader::with_capacity(1, File::open("tests/test.jpg").unwrap());
    let dinfo = Decompress::builder().from_reader(tricky_buf).unwrap();
    assert_eq!(0, dinfo.markers().count());

    let res = dinfo.rgb().unwrap().read_scanlines::<[u8; 3]>().unwrap();
    assert_eq!(res.len(), 45*30);

    let dinfo = Decompress::builder().with_markers(&[]).from_path("tests/test.jpg").unwrap();
    assert_eq!(0, dinfo.markers().count());
}

#[test]
fn read_file_rgb() {
    use crate::colorspace::ColorSpace;
    use crate::colorspace::ColorSpaceExt;
    use std::fs::File;
    use std::io::Read;

    let data = std::fs::read("tests/test.jpg").unwrap();
    let dinfo = Decompress::builder().with_markers(ALL_MARKERS).from_mem(&data[..]).unwrap();

    assert_eq!(ColorSpace::JCS_YCbCr, dinfo.color_space());

    assert_eq!(1, dinfo.markers().count());

    let mut dinfo = dinfo.rgb().unwrap();
    assert_eq!(ColorSpace::JCS_RGB, dinfo.color_space());
    assert_eq!(dinfo.components().len(), dinfo.color_space().num_components() as usize);

    let bitmap: Vec<[u8; 3]> = dinfo.read_scanlines().unwrap();
    assert_eq!(bitmap.len(), 45 * 30);

    assert!(!bitmap.contains(&[0; 3]));

    dinfo.finish().unwrap();
}

#[test]
fn drops_reader() {
    #[repr(align(1024))]
    struct CountsDrops<'a, R> {drop_count: &'a mut u8, reader: R}

    impl<R> Drop for CountsDrops<'_, R> {
        fn drop(&mut self) {
            assert!(self as *mut _ as usize % 1024 == 0); // alignment
            *self.drop_count += 1;
        }
    }
    impl<R: io::Read> io::Read for CountsDrops<'_, R> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> { self.reader.read(buf) }
    }
    let mut drop_count = 0;
    let r = Decompress::builder().from_reader(BufReader::new(CountsDrops {
        drop_count: &mut drop_count,
        reader: File::open("tests/test.jpg").unwrap(),
    })).unwrap();
    drop(r);
    assert_eq!(1, drop_count);
}
