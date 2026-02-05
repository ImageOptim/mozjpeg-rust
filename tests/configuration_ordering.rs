//! Tests demonstrating configuration ordering bugs in the mozjpeg API
//!
//! These tests expose issues where calling API methods in different orders
//! produces different results due to hidden side effects (e.g., jpeg_set_defaults()
//! resetting previously-set configuration).
//!
//! The goal is to demonstrate that ORDER MATTERS even when the same settings
//! are requested, which is a major footgun in the API.

use mozjpeg::{ColorSpace, Compress, ScanMode};

/// Helper to create a simple test image with some variation
fn create_test_image(width: usize, height: usize) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        for x in 0..width {
            pixels.push(((x + y) % 256) as u8); // R
            pixels.push((x % 256) as u8); // G
            pixels.push((y % 256) as u8); // B
        }
    }
    pixels
}

/// Helper to encode with given configuration
fn encode_jpeg(comp: Compress, pixels: &[u8]) -> Vec<u8> {
    let mut started = comp.start_compress(Vec::new()).unwrap();
    started.write_scanlines(pixels).unwrap();
    started.finish().unwrap()
}

#[test]
fn smoothing_preserved_after_fix() {
    // This test verifies that the smoothing_factor fix works correctly
    // On this branch (fix-smoothing-factor), set_scan_optimization_mode() now
    // preserves smoothing_factor regardless of call order

    let pixels = create_test_image(64, 64);

    // Order 1: Set smoothing AFTER scan optimization
    let mut comp1 = Compress::new(ColorSpace::JCS_RGB);
    comp1.set_size(64, 64);
    comp1.set_quality(85.0);
    comp1.set_scan_optimization_mode(ScanMode::Auto);
    comp1.set_smoothing_factor(50);
    let jpeg_after = encode_jpeg(comp1, &pixels);

    // Order 2: Set smoothing BEFORE scan optimization
    let mut comp2 = Compress::new(ColorSpace::JCS_RGB);
    comp2.set_size(64, 64);
    comp2.set_quality(85.0);
    comp2.set_smoothing_factor(50);
    comp2.set_scan_optimization_mode(ScanMode::Auto); // Should preserve smoothing now!
    let jpeg_before = encode_jpeg(comp2, &pixels);

    // Order 3: No smoothing (for reference)
    let mut comp3 = Compress::new(ColorSpace::JCS_RGB);
    comp3.set_size(64, 64);
    comp3.set_quality(85.0);
    comp3.set_scan_optimization_mode(ScanMode::Auto);
    let jpeg_no_smoothing = encode_jpeg(comp3, &pixels);

    // FIXED: Both orderings now produce identical output with smoothing
    assert_eq!(
        jpeg_after, jpeg_before,
        "FIXED: Order no longer matters for smoothing_factor!"
    );

    // Both should differ from no smoothing
    assert_ne!(
        jpeg_after, jpeg_no_smoothing,
        "Smoothed output should differ from non-smoothed"
    );
}

#[test]
fn subsampling_order_affects_output() {
    // This test demonstrates that set_scan_optimization_mode() resets subsampling factors

    let pixels = create_test_image(64, 64);

    // With lazy config (0.11+), use mutate_components_last() for order-independent subsampling
    // The callback runs AFTER all other config, including jpeg_set_defaults()

    // Order 1: Set subsampling callback, then scan optimization
    let mut comp1 = Compress::new(ColorSpace::JCS_RGB);
    comp1.set_size(64, 64);
    comp1.set_color_space(ColorSpace::JCS_YCbCr);
    comp1.set_quality(85.0);
    comp1.mutate_components_last(|comps| {
        comps[0].h_samp_factor = 2;
        comps[0].v_samp_factor = 1;
        comps[1].h_samp_factor = 1;
        comps[1].v_samp_factor = 1;
        comps[2].h_samp_factor = 1;
        comps[2].v_samp_factor = 1;
    });
    comp1.set_scan_optimization_mode(ScanMode::Auto);
    let jpeg_callback_first = encode_jpeg(comp1, &pixels);

    // Order 2: Set scan optimization, then subsampling callback
    let mut comp2 = Compress::new(ColorSpace::JCS_RGB);
    comp2.set_size(64, 64);
    comp2.set_color_space(ColorSpace::JCS_YCbCr);
    comp2.set_quality(85.0);
    comp2.set_scan_optimization_mode(ScanMode::Auto);
    comp2.mutate_components_last(|comps| {
        comps[0].h_samp_factor = 2;
        comps[0].v_samp_factor = 1;
        comps[1].h_samp_factor = 1;
        comps[1].v_samp_factor = 1;
        comps[2].h_samp_factor = 1;
        comps[2].v_samp_factor = 1;
    });
    let jpeg_callback_last = encode_jpeg(comp2, &pixels);

    // With lazy config and mutate_components_last(), order doesn't matter
    assert_eq!(
        jpeg_callback_first, jpeg_callback_last,
        "Order no longer matters with mutate_components_last() - both produce 4:2:2"
    );
}

#[test]
fn progressive_mode_order_affects_output() {
    // This test demonstrates that progressive mode settings can be affected by ordering
    // when combined with set_scan_optimization_mode()

    let pixels = create_test_image(64, 64);

    // Order 1: Progressive mode WITHOUT scan optimization
    let mut comp1 = Compress::new(ColorSpace::JCS_RGB);
    comp1.set_size(64, 64);
    comp1.set_quality(85.0);
    comp1.set_progressive_mode();
    let jpeg_progressive_clean = encode_jpeg(comp1, &pixels);

    // Order 2: Set progressive BEFORE scan optimization (may be reset)
    let mut comp2 = Compress::new(ColorSpace::JCS_RGB);
    comp2.set_size(64, 64);
    comp2.set_quality(85.0);
    comp2.set_progressive_mode(); // Set BEFORE
    comp2.set_scan_optimization_mode(ScanMode::Auto); // May reset progressive settings!
    let jpeg_progressive_before = encode_jpeg(comp2, &pixels);

    // Order 3: Set progressive AFTER scan optimization (correct order)
    let mut comp3 = Compress::new(ColorSpace::JCS_RGB);
    comp3.set_size(64, 64);
    comp3.set_quality(85.0);
    comp3.set_scan_optimization_mode(ScanMode::Auto);
    comp3.set_progressive_mode(); // Set AFTER
    let jpeg_progressive_after = encode_jpeg(comp3, &pixels);

    // Order 4: Baseline (no progressive) for reference
    let mut comp4 = Compress::new(ColorSpace::JCS_RGB);
    comp4.set_size(64, 64);
    comp4.set_quality(85.0);
    let jpeg_baseline = encode_jpeg(comp4, &pixels);

    // Check if ordering matters
    println!("\nProgressive mode test results:");
    println!(
        "  Progressive clean: {} bytes",
        jpeg_progressive_clean.len()
    );
    println!(
        "  Progressive before scan_opt: {} bytes",
        jpeg_progressive_before.len()
    );
    println!(
        "  Progressive after scan_opt: {} bytes",
        jpeg_progressive_after.len()
    );
    println!("  Baseline: {} bytes", jpeg_baseline.len());

    if jpeg_progressive_clean == jpeg_baseline {
        println!("  NOTE: Progressive and baseline produce identical output (unexpected!)");
        println!("        This may indicate progressive mode isn't being applied or");
        println!("        the test image is too simple to show differences");
    } else {
        println!("  ✓ Progressive differs from baseline");
    }

    if jpeg_progressive_before != jpeg_progressive_after {
        println!("  BUG: Progressive mode order matters when using set_scan_optimization_mode()!");
    } else {
        println!("  OK: Progressive mode ordering doesn't affect output");
    }
}
