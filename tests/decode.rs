use mozjpeg::*;

#[test]
fn decode_test_rgb() {
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

    let _buf = image.read_scanlines::<[u8; 3]>().unwrap();
}

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

    let _buf = image.read_scanlines::<[u8; 4]>().unwrap();
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
    let buf = image.read_scanlines_flat().unwrap();

    assert_eq!(buf.len(), buf_size);
}

#[test]
fn decode_test_rgba_flat() {
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

    let buf_size = image.min_flat_buffer_size();
    let buf = image.read_scanlines_flat().unwrap();
    assert_eq!(buf.len(), buf_size);
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
