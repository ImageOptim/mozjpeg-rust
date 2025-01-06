use mozjpeg::*;

pub fn decompress_jpeg(jpeg: &[u8]) -> Vec<Vec<u8>> {
    let decomp = mozjpeg::Decompress::new_mem(jpeg).unwrap();

    let mut bitmaps:Vec<_> = decomp.components().iter().map(|c|{
        Vec::with_capacity(c.row_stride() * c.col_stride())
    }).collect();

    let mut decomp = decomp.raw().unwrap();
    {
        let mut bitmap_refs: Vec<_> = bitmaps.iter_mut().collect();
        decomp.read_raw_data(&mut bitmap_refs);
        decomp.finish().unwrap();
    }

    bitmaps
}

#[test]
fn color_jpeg() {
    for size in 1..64 {
        let mut comp = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);

        comp.set_scan_optimization_mode(mozjpeg::ScanMode::AllComponentsTogether);
        comp.set_size(size, size);
        let mut comp = comp.start_compress(Vec::new()).unwrap();

        let lines = vec![128; size * size * 3];
        comp.write_scanlines(&lines[..]).unwrap();

        let jpeg = comp.finish().unwrap();
        assert!(!jpeg.is_empty());
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

        let mut comp = comp.start_compress(Vec::new()).unwrap();

        let rounded_size = (size + 7) / 8 * 8;
        let t = vec![128; rounded_size * rounded_size];
        let components = [&t[..], &t[..], &t[..]];
        comp.write_raw_data(&components[..]);

        let jpeg = comp.finish().unwrap();
        assert!(!jpeg.is_empty());
        decompress_jpeg(&jpeg);
    }
}

#[test]
fn decode_test() {
    let d = Decompress::with_markers(ALL_MARKERS)
        .from_path("tests/test.jpg")
        .unwrap();

    assert_eq!(45, d.width());
    assert_eq!(30, d.height());
    assert_eq!(1.0, d.gamma());
    assert_eq!(ColorSpace::JCS_YCbCr, d.color_space());
    assert_eq!(1, d.markers().count());

    let image = d.rgb().unwrap();
    assert_eq!(45, image.width());
    assert_eq!(30, image.height());
    assert_eq!(ColorSpace::JCS_RGB, image.color_space());
}

#[test]
fn decode_failure_test() {
    assert!(std::panic::catch_unwind(|| {
        Decompress::with_markers(ALL_MARKERS)
            .from_path("tests/test.rs")
            .unwrap();
    })
    .is_err());
}

#[test]
fn roundtrip() {
    let decoded = decode_jpeg(&std::fs::read("tests/test.jpg").unwrap());
    decode_jpeg(&encode_subsampled_jpeg(decoded));
}

#[test]
fn icc_profile() {
    let decoded = decode_jpeg(&std::fs::read("tests/test.jpg").unwrap());
    let img = encode_jpeg_with_icc_profile(decoded);
    let d = Decompress::with_markers(ALL_MARKERS)
        .from_mem(&img)
        .unwrap();

    assert_eq!(45, d.width());
    assert_eq!(30, d.height());
    assert_eq!(1.0, d.gamma());
    assert_eq!(ColorSpace::JCS_YCbCr, d.color_space());
    assert_eq!(10, d.markers().count()); // 9 for icc profile

    // silly checks
    d.markers().skip(1).for_each(|marker| {
        assert!(marker.data.starts_with(b"ICC_PROFILE\0"));
    });

    let image = d.rgb().unwrap();
    assert_eq!(45, image.width());
    assert_eq!(30, image.height());
    assert_eq!(ColorSpace::JCS_RGB, image.color_space());
}

fn encode_subsampled_jpeg((width, height, data): (usize, usize, Vec<[u8; 3]>)) -> Vec<u8> {
    let mut encoder = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
    encoder.set_size(width, height);

    encoder.set_color_space(mozjpeg::ColorSpace::JCS_YCbCr);
    {
        let comp = encoder.components_mut();
        comp[0].h_samp_factor = 1;
        comp[0].v_samp_factor = 1;

        let (h, v) = (2, 2); // CbCr420 subsampling factors
                             // 0 - Y, 1 - Cb, 2 - Cr, 3 - K
        comp[1].h_samp_factor = h;
        comp[1].v_samp_factor = v;
        comp[2].h_samp_factor = h;
        comp[2].v_samp_factor = v;
    }

    let mut encoder = encoder.start_compress(Vec::new()).unwrap();
    let _ = encoder.write_scanlines(bytemuck::cast_slice(&data));
    encoder.finish().unwrap()
}

fn encode_jpeg_with_icc_profile((width, height, data): (usize, usize, Vec<[u8; 3]>)) -> Vec<u8> {
    let mut encoder = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
    encoder.set_size(width, height);

    encoder.set_color_space(mozjpeg::ColorSpace::JCS_YCbCr);

    let mut encoder = encoder.start_compress(Vec::new()).unwrap();

    encoder.write_icc_profile(&std::fs::read("tests/test.icc").unwrap());

    let _ = encoder.write_scanlines(bytemuck::cast_slice(&data));
    encoder.finish().unwrap()
}

fn decode_jpeg(buffer: &[u8]) -> (usize, usize, Vec<[u8; 3]>) {
    let mut decoder = match mozjpeg::Decompress::new_mem(buffer).unwrap().image().unwrap() {
        mozjpeg::decompress::Format::RGB(d) => d,
        _ => unimplemented!(),
    };

    let width = decoder.width();
    let height = decoder.height();

    let image = decoder.read_scanlines::<[u8; 3]>().unwrap();

    (width, height, image)
}
