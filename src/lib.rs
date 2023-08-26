#![allow(unused_attributes)]
#![allow(unused_imports)]

use mozjpeg_sys as ffi;

pub use crate::colorspace::ColorSpace;
pub use crate::colorspace::ColorSpaceExt;
pub use crate::component::CompInfo;
pub use crate::component::CompInfoExt;
pub use crate::compress::Compress;
pub use crate::compress::ScanMode;
pub use crate::decompress::{DctMethod, Format};
pub use crate::decompress::{Decompress, ALL_MARKERS, NO_MARKERS};
use crate::ffi::boolean;
use crate::ffi::jpeg_common_struct;
use crate::ffi::jpeg_compress_struct;
pub use crate::ffi::DCTSIZE;
use crate::ffi::JDIMENSION;
pub use crate::ffi::JPEG_LIB_VERSION;
use crate::ffi::J_BOOLEAN_PARAM;
use crate::ffi::J_INT_PARAM;
pub use crate::marker::Marker;

use libc::free;
use std::cmp::min;
use std::mem;
use std::os::raw::{c_int, c_uchar, c_ulong, c_void};
use std::ptr;
use std::slice;

mod colorspace;
mod component;
mod compress;
pub mod decompress;
mod errormgr;
mod marker;
/// Quantization table presets from MozJPEG
pub mod qtable;
mod vec;
mod readsrc;

#[test]
fn recompress() {
    use crate::colorspace::ColorSpace;
    use crate::colorspace::ColorSpaceExt;
    use std::fs::File;
    use std::io::Read;
    use std::io::Write;

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

        #[allow(deprecated)] {
            cinfo.set_gamma(1.0);
        }

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

        assert!(cinfo.write_raw_data(&bitmaps.iter().map(|c| &c[..]).collect::<Vec<_>>()));

        cinfo.finish_compress();

        return cinfo.data_to_vec().unwrap();
    }

    let data1 = &write_jpeg(&bitmaps, &samp_factors, (1., 1.));
    let data1_len = data1.len();
    let data2 = &write_jpeg(&bitmaps, &samp_factors, (0.5, 0.5));
    let data2_len = data2.len();

    File::create("testout-r1.jpg").unwrap().write_all(data1).unwrap();
    File::create("testout-r2.jpg").unwrap().write_all(data2).unwrap();

    assert!(data1_len > data2_len);
}

#[cold]
fn fail(cinfo: &mut jpeg_common_struct, code: c_int) -> ! {
    unsafe {
        let err = &mut *cinfo.err;
        err.msg_code = code;
        if let Some(e) = err.error_exit {
            (e)(cinfo); // should have been defined as !
        }
        std::process::abort();
    }
}

fn warn(cinfo: &mut jpeg_common_struct, code: c_int) {
    unsafe {
        let err = &mut *cinfo.err;
        err.msg_code = code;
        if let Some(e) = err.emit_message {
            (e)(cinfo, -1);
        }
    }
}
