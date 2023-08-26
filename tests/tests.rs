use mozjpeg::*;

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

fn encode_subsampled_jpeg((width, height, data): (usize, usize, Vec<[u8; 3]>)) -> Vec<u8> {

    let mut encoder = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
    encoder.set_mem_dest();
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

    encoder.start_compress();
    let _ = encoder.write_scanlines(bytemuck::cast_slice(&data));
    encoder.finish_compress();

    encoder.data_to_vec().unwrap()
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
