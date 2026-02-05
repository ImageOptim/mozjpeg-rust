//! Test edge cases and invalid usage patterns to determine error handling needs.
//! These tests document current behavior - panics vs errors vs success.

use mozjpeg::*;
use std::panic::catch_unwind;

fn test_returns_error<F: FnOnce() -> Result<Vec<u8>, std::io::Error> + std::panic::UnwindSafe>(
    f: F,
) -> Option<String> {
    match catch_unwind(f) {
        Ok(Ok(_)) => None,
        Ok(Err(e)) => Some(e.to_string()),
        Err(_) => Some("PANIC".to_string()),
    }
}

#[test]
fn empty_scanlines_then_finish_panics() {
    // Writing empty scanlines then finishing without data is invalid
    // libjpeg requires all scanlines to be written
    let result = test_returns_error(|| {
        let mut comp = Compress::new(ColorSpace::JCS_RGB);
        comp.set_size(64, 64);
        let mut started = comp.start_compress(Vec::new())?;
        started.write_scanlines(&[])?; // no-op, but image incomplete
        started.finish() // Fails because image data is incomplete
    });
    println!("Empty scanlines then finish: {:?}", result);
    // This correctly fails - libjpeg requires complete image data
    assert!(result.is_some(), "Should fail with incomplete image");
}

#[test]
fn zero_width_returns_error() {
    let result = test_returns_error(|| {
        let mut comp = Compress::new(ColorSpace::JCS_RGB);
        comp.set_size(0, 64);
        let started = comp.start_compress(Vec::new())?;
        started.finish()
    });
    println!("Zero width: {:?}", result);
    // Zero dimensions return a proper error
    assert!(result.is_some(), "Zero width should return error");
    assert!(
        result.as_ref().unwrap().contains("invalid dimensions"),
        "Error should mention invalid dimensions: {:?}",
        result
    );
}

#[test]
fn zero_height_returns_error() {
    let result = test_returns_error(|| {
        let mut comp = Compress::new(ColorSpace::JCS_RGB);
        comp.set_size(64, 0);
        let started = comp.start_compress(Vec::new())?;
        started.finish()
    });
    println!("Zero height: {:?}", result);
    // Zero dimensions return a proper error
    assert!(result.is_some(), "Zero height should return error");
    assert!(
        result.as_ref().unwrap().contains("invalid dimensions"),
        "Error should mention invalid dimensions: {:?}",
        result
    );
}

#[test]
fn finish_without_writing_any_data() {
    let result = test_returns_error(|| {
        let mut comp = Compress::new(ColorSpace::JCS_RGB);
        comp.set_size(8, 8);
        let started = comp.start_compress(Vec::new())?;
        started.finish()
    });
    println!("Finish without data: {:?}", result);
    // This panics in libjpeg - user error, not much we can do
}

#[test]
fn partial_row_too_short() {
    let result = test_returns_error(|| {
        let mut comp = Compress::new(ColorSpace::JCS_RGB);
        comp.set_size(64, 64);
        let mut started = comp.start_compress(Vec::new())?;
        started.write_scanlines(&[128u8; 10])?; // need 64*3=192
        started.finish()
    });
    println!("Partial row (too short): {:?}", result);
    // Short data may cause undefined behavior - libjpeg reads past buffer
}

#[test]
fn stride_smaller_than_width() {
    let result = test_returns_error(|| {
        let mut comp = Compress::new(ColorSpace::JCS_RGB);
        comp.set_size(64, 64);
        let mut started = comp.start_compress(Vec::new())?;
        started.write_scanlines_strided(&[128u8; 1000], 10)?;
        started.finish()
    });
    println!("Stride too small: {:?}", result);
    // This should return an error - and it does!
    assert!(result.is_some(), "Stride too small should return error");
    assert_ne!(
        result.as_deref(),
        Some("PANIC"),
        "Should be error, not panic"
    );
}

#[test]
fn invalid_sampling_factor_zero_returns_error() {
    let result = test_returns_error(|| {
        let mut comp = Compress::new(ColorSpace::JCS_RGB);
        comp.set_size(64, 64);
        comp.set_color_space(ColorSpace::JCS_YCbCr);
        comp.mutate_components_last(|c| c[0].h_samp_factor = 0);
        comp.start_compress(Vec::new())?.finish()
    });
    println!("h_samp_factor = 0: {:?}", result);
    // Invalid sampling factors now return proper error
    assert!(
        result.is_some(),
        "Invalid sampling factor should return error"
    );
    assert!(
        result.as_ref().unwrap().contains("sampling factors"),
        "Error should mention sampling factors: {:?}",
        result
    );
}

#[test]
fn marker_mid_stream_is_invalid() {
    // According to JPEG spec, markers can only appear at specific points.
    // Mid-scan markers (between partial scanline writes) are NOT valid.
    let result = test_returns_error(|| {
        let mut comp = Compress::new(ColorSpace::JCS_RGB);
        comp.set_size(8, 8);
        let mut started = comp.start_compress(Vec::new())?;
        started.write_scanlines(&[128u8; 8 * 4 * 3])?; // 4 of 8 rows
        started.write_marker(Marker::COM, b"mid-stream"); // Invalid position!
        started.write_scanlines(&[128u8; 8 * 4 * 3])?; // remaining 4
        started.finish()
    });
    println!("Marker mid-stream: {:?}", result);
    // This correctly fails - mid-scan markers are invalid
    assert!(result.is_some(), "Mid-stream markers should be rejected");
}

#[test]
fn marker_before_scanlines_works() {
    // Markers before scan data are valid
    let result = test_returns_error(|| {
        let mut comp = Compress::new(ColorSpace::JCS_RGB);
        comp.set_size(8, 8);
        let mut started = comp.start_compress(Vec::new())?;
        started.write_marker(Marker::COM, b"header-comment");
        started.write_scanlines(&[128u8; 8 * 8 * 3])?;
        started.finish()
    });
    println!("Marker before scanlines: {:?}", result);
    assert!(
        result.is_none(),
        "Marker before scanlines should work: {:?}",
        result
    );
}

#[test]
fn very_large_dimensions_returns_error() {
    let result = test_returns_error(|| {
        let mut comp = Compress::new(ColorSpace::JCS_RGB);
        comp.set_size(100000, 100000);
        comp.start_compress(Vec::new())?.finish()
    });
    println!("Very large dimensions: {:?}", result);
    // Dimensions exceeding JPEG max (65535) now return proper error
    assert!(
        result.is_some(),
        "Very large dimensions should return error"
    );
    assert!(
        result.as_ref().unwrap().contains("exceed JPEG maximum"),
        "Error should mention JPEG maximum: {:?}",
        result
    );
}

#[test]
fn smoothing_max_value() {
    let result = test_returns_error(|| {
        let mut comp = Compress::new(ColorSpace::JCS_RGB);
        comp.set_size(8, 8);
        comp.set_smoothing_factor(255); // max u8
        let mut started = comp.start_compress(Vec::new())?;
        started.write_scanlines(&[128u8; 8 * 8 * 3])?;
        started.finish()
    });
    println!("Smoothing factor 255: {:?}", result);
    // Smoothing factor is clamped by libjpeg
    assert!(result.is_none(), "Max smoothing should work");
}

#[test]
fn quality_out_of_range() {
    // Negative quality
    let result1 = test_returns_error(|| {
        let mut comp = Compress::new(ColorSpace::JCS_RGB);
        comp.set_size(8, 8);
        comp.set_quality(-50.0);
        let mut started = comp.start_compress(Vec::new())?;
        started.write_scanlines(&[128u8; 8 * 8 * 3])?;
        started.finish()
    });
    println!("Negative quality: {:?}", result1);

    // Quality > 100
    let result2 = test_returns_error(|| {
        let mut comp = Compress::new(ColorSpace::JCS_RGB);
        comp.set_size(8, 8);
        comp.set_quality(150.0);
        let mut started = comp.start_compress(Vec::new())?;
        started.write_scanlines(&[128u8; 8 * 8 * 3])?;
        started.finish()
    });
    println!("Quality > 100: {:?}", result2);

    // Both should work - libjpeg clamps values
    assert!(result1.is_none(), "Negative quality should be clamped");
    assert!(result2.is_none(), "Quality > 100 should be clamped");
}

#[test]
fn write_more_rows_than_height() {
    let result = test_returns_error(|| {
        let mut comp = Compress::new(ColorSpace::JCS_RGB);
        comp.set_size(4, 4); // 4 rows
        let mut started = comp.start_compress(Vec::new())?;
        let pixels = vec![128u8; 4 * 100 * 3]; // 100 rows
        started.write_scanlines(&pixels)?;
        started.finish()
    });
    println!("More rows than height: {:?}", result);
    // libjpeg just stops reading at image_height
}

#[test]
fn normal_operation_works() {
    // Sanity check - normal operation should work
    let result = test_returns_error(|| {
        let mut comp = Compress::new(ColorSpace::JCS_RGB);
        comp.set_size(8, 8);
        comp.set_quality(80.0);
        let mut started = comp.start_compress(Vec::new())?;
        started.write_scanlines(&[128u8; 8 * 8 * 3])?;
        started.finish()
    });
    assert!(
        result.is_none(),
        "Normal operation should succeed: {:?}",
        result
    );
}
