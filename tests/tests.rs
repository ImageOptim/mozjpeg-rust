#![allow(deprecated)]

use mozjpeg::*;

pub fn decompress_jpeg(jpeg: &[u8]) -> Vec<Vec<u8>> {
    let decomp = mozjpeg::Decompress::new_mem(jpeg).unwrap();

    let mut bitmaps: Vec<_> = decomp
        .components()
        .iter()
        .map(|c| Vec::with_capacity(c.row_stride() * c.col_stride()))
        .collect();

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

// ============================================================================
// force_8bit_quantization tests
// ============================================================================

#[test]
fn force_8bit_false_matches_set_quality() {
    // set_force_8bit_quantization(false) is the default, so should match set_quality()
    for &quality in &[10.0, 50.0, 85.0, 95.0] {
        let size = 32;
        let pixels = vec![128u8; size * size * 3];

        // Default (no force_8bit set)
        let mut comp = Compress::new(ColorSpace::JCS_RGB);
        comp.set_size(size, size);
        comp.set_quality(quality);
        let mut started = comp.start_compress(Vec::new()).unwrap();
        started.write_scanlines(&pixels).unwrap();
        let jpeg_default = started.finish().unwrap();

        // Explicitly set force_8bit=false
        let mut comp = Compress::new(ColorSpace::JCS_RGB);
        comp.set_size(size, size);
        comp.set_quality(quality);
        comp.set_force_8bit_quantization(false);
        let mut started = comp.start_compress(Vec::new()).unwrap();
        started.write_scanlines(&pixels).unwrap();
        let jpeg_explicit = started.finish().unwrap();

        assert_eq!(
            jpeg_default, jpeg_explicit,
            "explicit force_8bit=false should match default at quality {quality}"
        );
    }
}

#[test]
fn force_8bit_true_clamps_at_low_quality() {
    // At low quality, quantization values exceed 255. Clamping produces different output.
    let size = 32;
    let pixels = vec![128u8; size * size * 3];

    let mut comp = Compress::new(ColorSpace::JCS_RGB);
    comp.set_size(size, size);
    comp.set_quality(10.0);
    let mut started = comp.start_compress(Vec::new()).unwrap();
    started.write_scanlines(&pixels).unwrap();
    let jpeg_unclamped = started.finish().unwrap();

    let mut comp = Compress::new(ColorSpace::JCS_RGB);
    comp.set_size(size, size);
    comp.set_quality(10.0);
    comp.set_force_8bit_quantization(true);
    let mut started = comp.start_compress(Vec::new()).unwrap();
    started.write_scanlines(&pixels).unwrap();
    let jpeg_clamped = started.finish().unwrap();

    assert_ne!(
        jpeg_unclamped, jpeg_clamped,
        "force_8bit at quality 10 should produce different output (16-bit vs 8-bit DQT)"
    );
    // Clamped version is slightly smaller (8-bit DQT markers vs 16-bit)
    assert!(
        jpeg_clamped.len() < jpeg_unclamped.len(),
        "8-bit DQT should be smaller than 16-bit DQT"
    );
}

#[test]
fn force_8bit_no_effect_at_high_quality() {
    // At high quality, all quantization values are <= 255 anyway.
    let size = 32;
    let pixels = vec![128u8; size * size * 3];

    let mut comp = Compress::new(ColorSpace::JCS_RGB);
    comp.set_size(size, size);
    comp.set_quality(85.0);
    let mut started = comp.start_compress(Vec::new()).unwrap();
    started.write_scanlines(&pixels).unwrap();
    let jpeg_unclamped = started.finish().unwrap();

    let mut comp = Compress::new(ColorSpace::JCS_RGB);
    comp.set_size(size, size);
    comp.set_quality(85.0);
    comp.set_force_8bit_quantization(true);
    let mut started = comp.start_compress(Vec::new()).unwrap();
    started.write_scanlines(&pixels).unwrap();
    let jpeg_clamped = started.finish().unwrap();

    assert_eq!(
        jpeg_unclamped, jpeg_clamped,
        "force_8bit should have no effect at quality 85 (all values already <= 255)"
    );
}

fn decode_jpeg(buffer: &[u8]) -> (usize, usize, Vec<[u8; 3]>) {
    let mut decoder = match mozjpeg::Decompress::new_mem(buffer)
        .unwrap()
        .image()
        .unwrap()
    {
        mozjpeg::decompress::Format::RGB(d) => d,
        _ => unimplemented!(),
    };

    let width = decoder.width();
    let height = decoder.height();

    let image = decoder.read_scanlines::<[u8; 3]>().unwrap();

    (width, height, image)
}

/// Test that smoothing_factor is preserved when calling set_scan_optimization_mode
/// Regression test for https://github.com/ImageOptim/mozjpeg-rust/issues/45
#[test]
fn smoothing_factor_preserved() {
    // Generate a dither-like pattern that benefits from smoothing
    let size = 64;
    let mut data = Vec::with_capacity(size * size * 3);
    for y in 0..size {
        for x in 0..size {
            let v = if ((x / 2) + (y / 2)) % 2 == 0 {
                80u8
            } else {
                176u8
            };
            data.push(v);
            data.push(v);
            data.push(v);
        }
    }

    // Compress without smoothing
    let size_no_smooth = {
        let mut comp = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
        comp.set_size(size, size);
        comp.set_quality(80.0);
        comp.set_smoothing_factor(0);
        comp.set_scan_optimization_mode(mozjpeg::ScanMode::Auto);
        let mut comp = comp.start_compress(Vec::new()).unwrap();
        comp.write_scanlines(&data).unwrap();
        comp.finish().unwrap().len()
    };

    // Compress with smoothing set BEFORE set_scan_optimization_mode
    let size_smooth_before = {
        let mut comp = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
        comp.set_size(size, size);
        comp.set_quality(80.0);
        comp.set_smoothing_factor(50);
        comp.set_scan_optimization_mode(mozjpeg::ScanMode::Auto);
        let mut comp = comp.start_compress(Vec::new()).unwrap();
        comp.write_scanlines(&data).unwrap();
        comp.finish().unwrap().len()
    };

    // Compress with smoothing set AFTER set_scan_optimization_mode
    let size_smooth_after = {
        let mut comp = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
        comp.set_size(size, size);
        comp.set_quality(80.0);
        comp.set_scan_optimization_mode(mozjpeg::ScanMode::Auto);
        comp.set_smoothing_factor(50);
        let mut comp = comp.start_compress(Vec::new()).unwrap();
        comp.write_scanlines(&data).unwrap();
        comp.finish().unwrap().len()
    };

    // Smoothing should reduce file size for high-frequency content
    assert!(
        size_smooth_after < size_no_smooth,
        "smoothing should reduce size: {} < {}",
        size_smooth_after,
        size_no_smooth
    );

    // Both orderings should produce the same result (this was the bug)
    assert_eq!(
        size_smooth_before, size_smooth_after,
        "smoothing_factor should work regardless of call order: {} != {}",
        size_smooth_before, size_smooth_after
    );
}
