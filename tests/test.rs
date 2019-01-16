extern crate mozjpeg;
use mozjpeg::CompInfoExt;

pub fn decompress_jpeg(jpeg: &[u8]) -> Vec<Vec<u8>> {
    let decomp = mozjpeg::Decompress::new_mem(jpeg).unwrap();

    let mut bitmaps:Vec<_> = decomp.components().iter().map(|c|{
        Vec::with_capacity(c.row_stride() * c.col_stride())
    }).collect();

    let mut decomp = decomp.raw().unwrap();
    {
        let mut bitmap_refs:Vec<_> = bitmaps.iter_mut().collect();
        decomp.read_raw_data(&mut bitmap_refs);
        decomp.finish_decompress();
    }

    return bitmaps;
}

#[test]
fn color_jpeg() {
    for size in 1..64 {
        let mut comp = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);

        comp.set_scan_optimization_mode(mozjpeg::ScanMode::AllComponentsTogether);
        comp.set_size(size, size);
        comp.set_mem_dest();
        comp.start_compress();

        let lines = vec![128; size*size*3];
        comp.write_scanlines(&lines[..]);

        comp.finish_compress();
        let jpeg = comp.data_to_vec().unwrap();

        decompress_jpeg(&jpeg);
    }
}

#[test]
fn raw_jpeg() {
    for size in 1..64 {
        let mut comp = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_YCbCr);

        comp.set_scan_optimization_mode(mozjpeg::ScanMode::AllComponentsTogether);

        comp.set_raw_data_in(true);
        comp.set_size(size, size);

        comp.set_mem_dest();
        comp.start_compress();

        let rounded_size = (size+7)/8*8;
        let t = vec![128; rounded_size*rounded_size];
        let components = vec![&t[..], &t[..], &t[..]];
        comp.write_raw_data(&components[..]);

        comp.finish_compress();
        let jpeg = comp.data_to_vec().unwrap();

        decompress_jpeg(&jpeg);
    }
}
