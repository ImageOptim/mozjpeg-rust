#![allow(deprecated)]

use errormgr::ErrorMgr;
use errormgr::PanicingErrorMgr;
use component::CompInfoExt;
use component::CompInfo;
use marker::Marker;
use colorspace::ColorSpace;
use colorspace::ColorSpaceExt;
use qtable::QTable;
use ffi;
use ffi::JPEG_LIB_VERSION;
use ffi::J_INT_PARAM;
use ffi::J_BOOLEAN_PARAM;
use ffi::jpeg_compress_struct;
use ffi::boolean;
use ffi::DCTSIZE;
use ffi::JDIMENSION;
use std::os::raw::{c_int, c_uint, c_ulong, c_uchar};
use libc::free;
use libc::c_void;
use arrayvec::ArrayVec;
use std::slice;
use std::mem;
use std::ptr;
use std::cmp::min;

const MAX_MCU_HEIGHT: usize = 16;
const MAX_COMPONENTS: usize = 4;

/// Create a new JPEG file from pixels
///
/// Wrapper for `jpeg_compress_struct`
pub struct Compress {
    cinfo: jpeg_compress_struct,
    own_err: Box<ErrorMgr>,
    outbuffer: *mut c_uchar,
    outsize: c_ulong,
}

#[derive(Copy,Clone)]
pub enum ScanMode {
    AllComponentsTogether = 0,
    ScanPerComponent = 1,
    Auto = 2,
}

impl Compress {
    /// Compress image using input in this colorspace
    ///
    /// By default errors cause panic and unwind through the C code,
    /// which strictly speaking is not guaranteed to work in Rust (but seems to work fine, at least in x86/64).
    pub fn new(color_space: ColorSpace) -> Compress {
        Compress::new_err(<ErrorMgr as PanicingErrorMgr>::new(), color_space)
    }

    pub fn new_err(err: ErrorMgr, color_space: ColorSpace) -> Compress {
        unsafe {
            let mut newself = Compress{
                cinfo: mem::zeroed(),
                own_err: Box::new(err),
                outbuffer: ::std::ptr::null_mut(),
                outsize: 0,
            };

            newself.cinfo.common.err = &mut *newself.own_err;

            let s = mem::size_of_val(&newself.cinfo) as usize;
            ffi::jpeg_CreateCompress(&mut newself.cinfo, JPEG_LIB_VERSION, s);

            newself.cinfo.in_color_space = color_space;
            newself.cinfo.input_components = color_space.num_components() as c_int;
            ffi::jpeg_set_defaults(&mut newself.cinfo);

            newself
        }
    }

    pub fn start_compress(&mut self) {
        unsafe {
            ffi::jpeg_start_compress(&mut self.cinfo, true as boolean);
        }
    }

    pub fn write_marker(&mut self, marker: Marker, data: &[u8]) {
        unsafe {
            ffi::jpeg_write_marker(&mut self.cinfo, marker.into(), data.as_ptr(), data.len() as c_uint);

        }
    }

    /// Expose components for modification
    pub fn components_mut(&mut self) -> &mut [CompInfo] {
        unsafe {
            slice::from_raw_parts_mut(self.cinfo.comp_info, self.cinfo.num_components as usize)
        }
    }

    pub fn components(&self) -> &[CompInfo] {
        unsafe {
            slice::from_raw_parts(self.cinfo.comp_info, self.cinfo.num_components as usize)
        }
    }

    fn can_write_more_lines(&self) -> bool {
        self.cinfo.next_scanline < self.cinfo.image_height
    }

    pub fn write_scanlines(&mut self, image_src: &[u8]) -> bool {
        assert_eq!(0, self.cinfo.raw_data_in);
        assert!(self.cinfo.input_components > 0);
        assert!(self.cinfo.image_width > 0);

        let byte_width = self.cinfo.image_width as usize * self.cinfo.input_components as usize;
        let mut row_pointers = ArrayVec::<[_; MAX_MCU_HEIGHT]>::new();
        for rows in image_src.chunks(row_pointers.capacity() * byte_width) {
            for row in rows.chunks(byte_width) {
                debug_assert!(row.len() == byte_width);
                row_pointers.push(row.as_ptr());
            }

            unsafe {
                let rows_written = ffi::jpeg_write_scanlines(&mut self.cinfo, row_pointers.as_ptr(), row_pointers.len() as u32) as usize;
                if rows_written < rows.len() {
                    return false;
                }
            }
        }
        return true;
    }

    pub fn write_raw_data(&mut self, image_src: &[&[u8]]) -> bool {
        if 0 == self.cinfo.raw_data_in {
            panic!("Raw data not set");
        }

        let mcu_height = self.cinfo.max_v_samp_factor as usize * DCTSIZE;
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

        let mut start_row = self.cinfo.next_scanline as usize;
        while self.can_write_more_lines() {
            unsafe {
                let mut row_ptrs = [[ptr::null::<u8>(); MAX_MCU_HEIGHT]; MAX_COMPONENTS];
                let mut comp_ptrs = [ptr::null::<*const u8>(); MAX_COMPONENTS];

                for (ci, comp_info) in self.components().iter().enumerate() {

                    let row_stride = comp_info.row_stride();

                    let input_height = image_src[ci].len() / row_stride;

                    let comp_start_row = start_row * comp_info.v_samp_factor as usize / self.cinfo.max_v_samp_factor as usize;
                    let comp_height = min(input_height - comp_start_row, DCTSIZE * comp_info.v_samp_factor as usize);
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

                let rows_written = ffi::jpeg_write_raw_data(&mut self.cinfo, comp_ptrs.as_ptr(), mcu_height as u32) as usize;
                if 0 == rows_written {
                    return false;
                }
                start_row += rows_written;
            }
        }
        return true;
    }

    pub fn set_color_space(&mut self, color_space: ColorSpace) {
        self.cinfo.input_components = color_space.num_components() as c_int;
        unsafe {
            ffi::jpeg_set_colorspace(&mut self.cinfo, color_space);
        }
    }

    pub fn set_size(&mut self, width: usize, height: usize) {
        self.cinfo.image_width = width as JDIMENSION;
        self.cinfo.image_height = height as JDIMENSION;
    }

    pub fn set_gamma(&mut self, gamma: f64) {
        self.cinfo.input_gamma = gamma;
    }

    pub fn set_optimize_scans(&mut self, opt: bool) {
        unsafe {
            ffi::jpeg_c_set_bool_param(&mut self.cinfo, J_BOOLEAN_PARAM::JBOOLEAN_OPTIMIZE_SCANS, opt as boolean);
        }
        if !opt {
            self.cinfo.scan_info = ptr::null();
        }
    }

    /// Set to `false` to make files larger for no reason
    pub fn set_optimize_coding(&mut self, opt: bool) {
        self.cinfo.optimize_coding = opt as boolean;
    }

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

    pub fn set_scan_optimization_mode(&mut self, mode: ScanMode) {
        unsafe {
            ffi::jpeg_c_set_int_param(&mut self.cinfo, J_INT_PARAM::JINT_DC_SCAN_OPT_MODE, mode as c_int);
            ffi::jpeg_set_defaults(&mut self.cinfo);
        }
    }

    /// Reset to libjpeg v6 settings
    pub fn set_fastest_defaults(&mut self) {
        unsafe {
            ffi::jpeg_c_set_int_param(&mut self.cinfo, J_INT_PARAM::JINT_COMPRESS_PROFILE, ffi::JINT_COMPRESS_PROFILE_VALUE::JCP_FASTEST as c_int);
            ffi::jpeg_set_defaults(&mut self.cinfo);
        }
    }

    pub fn set_raw_data_in(&mut self, opt: bool) {
        self.cinfo.raw_data_in = opt as boolean;
    }

    pub fn set_quality(&mut self, quality: f32) {
        unsafe {
            ffi::jpeg_set_quality(&mut self.cinfo, quality as c_int, false as boolean);
        }
    }

    pub fn set_luma_qtable(&mut self, qtable: &QTable) {
        unsafe {
            ffi::jpeg_add_quant_table(&mut self.cinfo, 0, qtable.as_ptr(), 100, 1);
        }
    }

    pub fn set_chroma_qtable(&mut self, qtable: &QTable) {
        unsafe {
            ffi::jpeg_add_quant_table(&mut self.cinfo, 1, qtable.as_ptr(), 100, 1);
        }
    }

    pub fn set_mem_dest(&mut self) {
        self.free_mem_dest();
        unsafe {
            ffi::jpeg_mem_dest(&mut self.cinfo, &mut self.outbuffer, &mut self.outsize);
        }
    }

    fn free_mem_dest(&mut self) {
        if !self.outbuffer.is_null() {
            unsafe {
                free(self.outbuffer as *mut c_void);
            }
            self.outbuffer = ptr::null_mut();
            self.outsize = 0;
        }
    }

    pub fn finish_compress(&mut self) {
        unsafe {
            ffi::jpeg_finish_compress(&mut self.cinfo);
        }
    }

    pub fn data_as_mut_slice(&mut self) -> Result<&[u8],()> {
        if self.outbuffer.is_null() || 0 == self.outsize {
            return Err(());
        }
        unsafe {
            Ok(slice::from_raw_parts(self.outbuffer, self.outsize as usize))
        }
    }

    pub fn data_to_vec(&mut self) -> Result<Vec<u8>,()> {
        if self.outbuffer.is_null() || 0 == self.outsize {
            return Err(());
        }
        unsafe {
            let res = Ok(slice::from_raw_parts(self.outbuffer, self.outsize as usize).to_vec());
            self.free_mem_dest();
            return res;
        }
    }
}

impl Drop for Compress {
    fn drop(&mut self) {
        self.free_mem_dest();
        unsafe {
            ffi::jpeg_destroy_compress(&mut self.cinfo);
        }
    }
}

#[test]
fn write_mem() {
    let mut cinfo = Compress::new(ColorSpace::JCS_YCbCr);

    assert_eq!(3, cinfo.components().len());

    cinfo.set_size(17, 33);

    cinfo.set_gamma(1.0);

    cinfo.set_progressive_mode();
    cinfo.set_scan_optimization_mode(ScanMode::AllComponentsTogether);

    cinfo.set_raw_data_in(true);

    cinfo.set_quality(88.);

    cinfo.set_mem_dest();

    for (c, samp) in cinfo.components_mut().iter_mut().zip(vec![2,1,1]) {
        c.v_samp_factor = samp;
        c.h_samp_factor = samp;
    }

    cinfo.start_compress();

    cinfo.write_marker(Marker::APP(2), "Hello World".as_bytes());

    assert_eq!(24, cinfo.components()[0].row_stride());
    assert_eq!(40, cinfo.components()[0].col_stride());
    assert_eq!(16, cinfo.components()[1].row_stride());
    assert_eq!(24, cinfo.components()[1].col_stride());
    assert_eq!(16, cinfo.components()[2].row_stride());
    assert_eq!(24, cinfo.components()[2].col_stride());

    let bitmaps = cinfo.components().iter().map(|c|{
        vec![128u8; c.row_stride() * c.col_stride()]
    }).collect::<Vec<_>>();

    assert!(cinfo.write_raw_data(&bitmaps.iter().map(|c|&c[..]).collect::<Vec<_>>()));

    cinfo.finish_compress();

    cinfo.data_to_vec().unwrap();
}
