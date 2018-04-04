#![allow(unused_attributes)]
#![allow(unused_imports)]
#[allow(deprecated)]

extern crate libc;
extern crate mozjpeg_sys as ffi;

pub use compress::Compress;
pub use compress::ScanMode;
pub use decompress::{Decompress, NO_MARKERS, ALL_MARKERS};
pub use decompress::Format;
pub use component::CompInfo;
pub use component::CompInfoExt;
pub use colorspace::ColorSpace;
pub use colorspace::ColorSpaceExt;
pub use marker::Marker;
pub use self::ffi::DCTSIZE;
pub use self::ffi::JPEG_LIB_VERSION;
use self::ffi::J_INT_PARAM;
use self::ffi::J_BOOLEAN_PARAM;
use self::ffi::JDIMENSION;
use self::ffi::jpeg_compress_struct;
use self::ffi::jpeg_common_struct;
use self::ffi::boolean;

use self::libc::{size_t, c_void, c_int, c_ulong, c_uchar};
use self::libc::free;
use ::std::slice;
use ::std::mem;
use ::std::ptr;
use ::std::cmp::min;

mod errormgr;
mod marker;
mod vec;
/// Quantization table presets from MozJPEG
pub mod qtable;
pub mod decompress;
mod compress;
mod component;
mod colorspace;

#[test]
fn recompress() {
    use std::fs::File;
    use std::io::Read;
    use std::io::Write;
    use colorspace::ColorSpace;
    use colorspace::ColorSpaceExt;

    let dinfo = Decompress::new_path("tests/test.jpg").unwrap();

    assert_eq!(1.0, dinfo.gamma());
    assert_eq!(ColorSpace::JCS_YCbCr, dinfo.color_space());
    assert_eq!(dinfo.components().len(), dinfo.color_space().num_components() as usize);

    let samp_factors = dinfo.components().iter().map(|c|c.v_samp_factor).collect::<Vec<_>>();

    assert_eq!((45, 30), dinfo.size());

    let mut dinfo = dinfo.raw().unwrap();

    let mut bitmaps = [&mut Vec::new(), &mut Vec::new(), &mut Vec::new()];
    dinfo.read_raw_data(&mut bitmaps);

    assert!(dinfo.finish_decompress());

    fn write_jpeg(bitmaps: &[&mut Vec<u8>; 3], samp_factors: &Vec<i32>, scale: (f32, f32)) -> Vec<u8> {

        let mut cinfo = Compress::new(ColorSpace::JCS_YCbCr);

        cinfo.set_size(45, 30);

        cinfo.set_gamma(1.0);

        cinfo.set_raw_data_in(true);

        cinfo.set_quality(100.);

        cinfo.set_luma_qtable(&qtable::AnnexK_Luma.scaled(99. * scale.0, 90. * scale.1));
        cinfo.set_chroma_qtable(&qtable::AnnexK_Chroma.scaled(100. * scale.0, 60. * scale.1));

        cinfo.set_mem_dest();

        for (c, samp) in cinfo.components_mut().iter_mut().zip(samp_factors) {
            c.v_samp_factor = *samp;
            c.h_samp_factor = *samp;
        }

        cinfo.start_compress();

        assert!(cinfo.write_raw_data(&bitmaps.iter().map(|c|&c[..]).collect::<Vec<_>>()));

        cinfo.finish_compress();

        return cinfo.data_to_vec().unwrap();
    }

    let data1 = &write_jpeg(&bitmaps, &samp_factors, (1.,1.));
    let data1_len = data1.len();
    let data2 = &write_jpeg(&bitmaps, &samp_factors, (0.5,0.5));
    let data2_len = data2.len();

    File::create("testout-r1.jpg").unwrap().write_all(data1).unwrap();
    File::create("testout-r2.jpg").unwrap().write_all(data2).unwrap();

    assert!(data1_len > data2_len);
}
