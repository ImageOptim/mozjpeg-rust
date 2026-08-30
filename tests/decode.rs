use mozjpeg::*;
use std::sync::LazyLock;

static RGB: LazyLock<Vec<[u8; 3]>> = LazyLock::new(|| {
    let d = Decompress::with_markers(ALL_MARKERS)
        .from_path("tests/test.jpg")
        .unwrap();

    assert_eq!(45, d.width());
    assert_eq!(30, d.height());
    assert_eq!(1.0, d.gamma());
    assert_eq!(ColorSpace::JCS_YCbCr, d.color_space());
    assert_eq!(1, d.markers().count());

    let mut image = d.rgb().unwrap();
    assert_eq!(45, image.width());
    assert_eq!(30, image.height());
    assert_eq!(ColorSpace::JCS_RGB, image.color_space());

    image.read_scanlines::<[u8; 3]>().unwrap()
});

#[test]
fn decode_test_rgba() {
    let d = Decompress::with_markers(ALL_MARKERS)
        .from_path("tests/test.jpg")
        .unwrap();

    assert_eq!(45, d.width());
    assert_eq!(30, d.height());
    assert_eq!(1.0, d.gamma());
    assert_eq!(ColorSpace::JCS_YCbCr, d.color_space());
    assert_eq!(1, d.markers().count());

    let mut image = d.rgba().unwrap();
    assert_eq!(45, image.width());
    assert_eq!(30, image.height());
    assert_eq!(ColorSpace::JCS_EXT_RGBA, image.color_space());

    let rgba = image.read_scanlines::<[u8; 4]>().unwrap();
    assert!(rgba.iter().map(|px| &px[..3]).eq(RGB.iter()));
}

#[test]
fn decode_test_argb() {
    let d = Decompress::with_markers(ALL_MARKERS)
        .from_path("tests/test.jpg")
        .unwrap();

    assert_eq!(45, d.width());
    assert_eq!(30, d.height());
    assert_eq!(1.0, d.gamma());
    assert_eq!(ColorSpace::JCS_YCbCr, d.color_space());
    assert_eq!(1, d.markers().count());

    let mut image = d.to_colorspace(ColorSpace::JCS_EXT_ARGB).unwrap();
    assert_eq!(45, image.width());
    assert_eq!(30, image.height());
    assert_eq!(ColorSpace::JCS_EXT_ARGB, image.color_space());

    let rgba = image.read_scanlines::<[u8; 4]>().unwrap();
    assert!(rgba.iter().map(|px| &px[1..]).eq(RGB.iter()));
}

#[test]
fn decode_test_rgb_flat() {
    let d = Decompress::with_markers(ALL_MARKERS)
        .from_path("tests/test.jpg")
        .unwrap();

    assert_eq!(45, d.width());
    assert_eq!(30, d.height());
    assert_eq!(1.0, d.gamma());
    assert_eq!(ColorSpace::JCS_YCbCr, d.color_space());
    assert_eq!(1, d.markers().count());

    let mut image = d.rgb().unwrap();
    assert_eq!(45, image.width());
    assert_eq!(30, image.height());
    assert_eq!(ColorSpace::JCS_RGB, image.color_space());

    let buf_size = image.min_flat_buffer_size();
    let buf = image.read_scanlines::<u8>().unwrap();

    assert_eq!(buf.len(), buf_size);

    assert!(buf.chunks_exact(3).eq(RGB.iter()));
}

#[test]
fn decode_test_rgba_flat() {
    for space in [ColorSpace::JCS_EXT_RGBA, ColorSpace::JCS_EXT_RGBX] {
        let d = Decompress::with_markers(ALL_MARKERS)
            .from_path("tests/test.jpg")
            .unwrap();

        assert_eq!(45, d.width());
        assert_eq!(30, d.height());
        assert_eq!(1.0, d.gamma());
        assert_eq!(ColorSpace::JCS_YCbCr, d.color_space());
        assert_eq!(1, d.markers().count());

        let mut image = d.to_colorspace(space).unwrap();
        assert_eq!(45, image.width());
        assert_eq!(30, image.height());
        assert_eq!(space, image.color_space());

        let buf_size = image.min_flat_buffer_size();
        let buf = image.read_scanlines::<u8>().unwrap();
        assert_eq!(buf.len(), buf_size);

        assert!(buf.chunks_exact(4).map(|px| &px[..3]).eq(RGB.iter()));
    }
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

/// Where the entropy-coded data of `tests/test.jpg` begins.
fn scan_start(jpeg: &[u8]) -> usize {
    let sos = jpeg.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
    let header_len = usize::from(u16::from_be_bytes([jpeg[sos + 2], jpeg[sos + 3]]));
    sos + 2 + header_len
}

fn decode_warnings(jpeg: &[u8]) -> Warnings {
    let mut image = Decompress::new_mem(jpeg).unwrap().rgb().unwrap();
    let pixels = image.read_scanlines::<[u8; 3]>().unwrap();
    // The point of the test: a damaged stream still yields a full-size image, so the
    // warnings are the only thing that distinguishes it from a sound one.
    assert_eq!(pixels.len(), 45 * 30);
    image.warnings()
}

#[test]
fn an_undamaged_stream_reports_no_warning() {
    let warnings = decode_warnings(&std::fs::read("tests/test.jpg").unwrap());
    assert!(warnings.is_empty());
    assert_eq!(warnings.total(), 0);
    assert_eq!(warnings.codes().count(), 0);
    assert!(!warnings.has_unnamed_code());
}

#[test]
fn a_corrupted_entropy_stream_reports_a_warning_code() {
    let mut jpeg = std::fs::read("tests/test.jpg").unwrap();
    let offset = scan_start(&jpeg) + 2;
    jpeg[offset] ^= 0xFF;

    let warnings = decode_warnings(&jpeg);
    assert!(!warnings.is_empty());
    assert_eq!(warnings.total(), 1);
    // JWRN_HUFF_BAD_CODE: libjpeg substituted a coefficient and carried on.
    assert_eq!(warnings.codes().collect::<Vec<_>>(), [118]);
    assert!(warnings.contains(118));
    assert!(!warnings.contains(120));
}

#[test]
fn a_truncated_stream_reports_running_out_of_data() {
    let jpeg = std::fs::read("tests/test.jpg").unwrap();
    let warnings = decode_warnings(&jpeg[..scan_start(&jpeg)]);
    assert!(!warnings.is_empty());
    // JWRN_HIT_MARKER then JWRN_JPEG_EOF. Notably not JWRN_JPEG_EOF alone, which is why a
    // caller cannot tell truncation from corruption by code set alone.
    assert_eq!(warnings.codes().collect::<Vec<_>>(), [117, 120]);
}
