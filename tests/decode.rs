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
