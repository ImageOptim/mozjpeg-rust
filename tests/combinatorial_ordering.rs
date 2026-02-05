#![allow(deprecated)]
//! Combinatorial testing of configuration call orderings.
//!
//! This test generates all possible orderings of configuration calls and
//! detects when order affects the output JPEG.
//!
//! NOTE: These tests compare output *bytes*, which can miss resets of settings
//! that don't affect the output for the given test pattern (e.g.,
//! `use_scans_in_trellis` without trellis quant enabled, or `pixel_density`
//! which is metadata-only). See `tests/reset_detection.rs` for struct-level
//! field inspection that catches ALL resets regardless of output impact.

use mozjpeg::{ColorSpace, Compress, ScanMode};
use std::collections::HashMap;

/// A configuration operation that can be applied to Compress
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ConfigOp {
    SetQuality(u8),        // Different quality values
    SetSmoothing(u8),      // Different smoothing values
    SetScanMode(ScanMode), // Different scan modes
    SetSubsampling422,     // 4:2:2 chroma subsampling
    SetSubsampling444,     // 4:4:4 (no subsampling)
    SetSubsampling420,     // 4:2:0 (default)
    SetProgressive,
    SetOptimizeCoding(bool),
    SetOptimizeScans(bool),
    SetUseScansInTrellis(bool),
    #[allow(dead_code)]
    SetColorSpace, // Set to YCbCr (reserved for future use)
    SetRawDataIn(bool), // Raw MCU mode
    SetPixelDensity,    // Set pixel density
}

impl ConfigOp {
    /// Apply this operation to a Compress instance
    fn apply(&self, comp: &mut Compress) {
        match self {
            ConfigOp::SetQuality(q) => {
                comp.set_quality(*q as f32);
            }
            ConfigOp::SetSmoothing(s) => {
                comp.set_smoothing_factor(*s);
            }
            ConfigOp::SetScanMode(mode) => {
                comp.set_scan_optimization_mode(*mode);
            }
            ConfigOp::SetSubsampling422 => {
                let comps = comp.components_mut();
                if comps.len() >= 3 {
                    comps[0].h_samp_factor = 2;
                    comps[0].v_samp_factor = 1;
                    comps[1].h_samp_factor = 1;
                    comps[1].v_samp_factor = 1;
                    comps[2].h_samp_factor = 1;
                    comps[2].v_samp_factor = 1;
                }
            }
            ConfigOp::SetSubsampling444 => {
                let comps = comp.components_mut();
                if comps.len() >= 3 {
                    comps[0].h_samp_factor = 1;
                    comps[0].v_samp_factor = 1;
                    comps[1].h_samp_factor = 1;
                    comps[1].v_samp_factor = 1;
                    comps[2].h_samp_factor = 1;
                    comps[2].v_samp_factor = 1;
                }
            }
            ConfigOp::SetSubsampling420 => {
                let comps = comp.components_mut();
                if comps.len() >= 3 {
                    comps[0].h_samp_factor = 2;
                    comps[0].v_samp_factor = 2;
                    comps[1].h_samp_factor = 1;
                    comps[1].v_samp_factor = 1;
                    comps[2].h_samp_factor = 1;
                    comps[2].v_samp_factor = 1;
                }
            }
            ConfigOp::SetProgressive => {
                comp.set_progressive_mode();
            }
            ConfigOp::SetOptimizeCoding(value) => {
                comp.set_optimize_coding(*value);
            }
            ConfigOp::SetOptimizeScans(value) => {
                comp.set_optimize_scans(*value);
            }
            ConfigOp::SetUseScansInTrellis(value) => {
                comp.set_use_scans_in_trellis(*value);
            }
            ConfigOp::SetColorSpace => {
                comp.set_color_space(ColorSpace::JCS_YCbCr);
            }
            ConfigOp::SetRawDataIn(value) => {
                comp.set_raw_data_in(*value);
            }
            ConfigOp::SetPixelDensity => {
                use mozjpeg::{PixelDensity, PixelDensityUnit};
                comp.set_pixel_density(PixelDensity {
                    unit: PixelDensityUnit::Inches,
                    x: 300,
                    y: 300,
                });
            }
        }
    }

    fn name(&self) -> String {
        match self {
            ConfigOp::SetQuality(q) => format!("quality({})", q),
            ConfigOp::SetSmoothing(s) => format!("smoothing({})", s),
            ConfigOp::SetScanMode(mode) => format!("scan_mode({:?})", mode),
            ConfigOp::SetSubsampling422 => "subsampling(4:2:2)".to_string(),
            ConfigOp::SetSubsampling444 => "subsampling(4:4:4)".to_string(),
            ConfigOp::SetSubsampling420 => "subsampling(4:2:0)".to_string(),
            ConfigOp::SetProgressive => "progressive".to_string(),
            ConfigOp::SetOptimizeCoding(v) => format!("optimize_coding({})", v),
            ConfigOp::SetOptimizeScans(v) => format!("optimize_scans({})", v),
            ConfigOp::SetUseScansInTrellis(v) => format!("use_scans_in_trellis({})", v),
            ConfigOp::SetColorSpace => "color_space(YCbCr)".to_string(),
            ConfigOp::SetRawDataIn(v) => format!("raw_data_in({})", v),
            ConfigOp::SetPixelDensity => "pixel_density(300dpi)".to_string(),
        }
    }
}

/// Generate all permutations of a slice
fn permutations<T: Clone>(items: &[T]) -> Vec<Vec<T>> {
    if items.is_empty() {
        return vec![vec![]];
    }
    if items.len() == 1 {
        return vec![vec![items[0].clone()]];
    }

    let mut result = Vec::new();
    for i in 0..items.len() {
        let mut remaining = items.to_vec();
        let item = remaining.remove(i);

        for mut perm in permutations(&remaining) {
            perm.insert(0, item.clone());
            result.push(perm);
        }
    }
    result
}

/// Encode a test image with the given configuration order
fn encode_with_order(ops: &[ConfigOp], pixels: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut comp = Compress::new(ColorSpace::JCS_RGB);
    comp.set_size(width, height);
    comp.set_color_space(ColorSpace::JCS_YCbCr);

    // Apply operations in order
    for op in ops {
        op.apply(&mut comp);
    }

    let mut started = comp.start_compress(Vec::new()).unwrap();
    started.write_scanlines(pixels).unwrap();
    started.finish().unwrap()
}

/// Create a test pattern image
fn create_test_pattern(width: usize, height: usize) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        for x in 0..width {
            pixels.push(((x + y) % 256) as u8);
            pixels.push((x % 256) as u8);
            pixels.push((y % 256) as u8);
        }
    }
    pixels
}

#[test]
fn test_all_orderings_basic_settings() {
    // Test a small set of settings that interact via set_scan_optimization_mode
    let settings = vec![
        ConfigOp::SetSmoothing(50),
        ConfigOp::SetScanMode(ScanMode::Auto),
        ConfigOp::SetSubsampling422,
    ];

    let width = 64;
    let height = 64;
    let pixels = create_test_pattern(width, height);

    println!(
        "\n=== Testing {} settings ({} permutations) ===",
        settings.len(),
        factorial(settings.len())
    );

    // Generate all permutations
    let perms = permutations(&settings);

    // Encode with each ordering and group by output
    let mut output_groups: HashMap<Vec<u8>, Vec<Vec<ConfigOp>>> = HashMap::new();

    for perm in perms {
        let output = encode_with_order(&perm, &pixels, width, height);
        output_groups.entry(output).or_default().push(perm);
    }

    println!(
        "\nFound {} unique outputs from {} orderings:",
        output_groups.len(),
        factorial(settings.len())
    );

    // Analyze groups
    for (i, (output, orderings)) in output_groups.iter().enumerate() {
        println!(
            "\n--- Output variant {} ({} bytes, {} orderings produce this) ---",
            i + 1,
            output.len(),
            orderings.len()
        );

        for (j, ordering) in orderings.iter().enumerate() {
            print!("  ");
            for (k, op) in ordering.iter().enumerate() {
                print!("{}", op.name());
                if k < ordering.len() - 1 {
                    print!(" → ");
                }
            }
            println!();
            if j >= 2 && orderings.len() > 3 {
                println!("  ... and {} more orderings", orderings.len() - 3);
                break;
            }
        }
    }

    // Assert if we found ordering-dependent behavior
    if output_groups.len() > 1 {
        println!(
            "\n⚠️  ORDERING MATTERS! Found {} different outputs",
            output_groups.len()
        );
        println!("This indicates configuration order affects the result.\n");
    } else {
        println!("\n✓ Order doesn't matter - all orderings produce identical output\n");
    }
}

#[test]
fn test_all_orderings_with_progressive() {
    // Test with progressive mode added
    let settings = vec![
        ConfigOp::SetSmoothing(50),
        ConfigOp::SetScanMode(ScanMode::Auto),
        ConfigOp::SetProgressive,
        ConfigOp::SetSubsampling422,
    ];

    let width = 64;
    let height = 64;
    let pixels = create_test_pattern(width, height);

    println!(
        "\n=== Testing {} settings ({} permutations) ===",
        settings.len(),
        factorial(settings.len())
    );

    let perms = permutations(&settings);
    let mut output_groups: HashMap<Vec<u8>, Vec<Vec<ConfigOp>>> = HashMap::new();

    for perm in perms {
        let output = encode_with_order(&perm, &pixels, width, height);
        output_groups.entry(output).or_default().push(perm);
    }

    println!(
        "\nFound {} unique outputs from {} orderings",
        output_groups.len(),
        factorial(settings.len())
    );

    // Show one example of each output variant
    for (i, (output, orderings)) in output_groups.iter().enumerate() {
        println!(
            "\n--- Output variant {} ({} bytes) ---",
            i + 1,
            output.len()
        );
        println!(
            "  Example: {}",
            orderings[0]
                .iter()
                .map(|op| op.name())
                .collect::<Vec<_>>()
                .join(" → ")
        );
        println!(
            "  ({} total orderings produce this output)",
            orderings.len()
        );
    }

    if output_groups.len() > 1 {
        println!("\n⚠️  ORDERING MATTERS!");
    }
}

#[test]
fn test_pairwise_interactions() {
    // Test all pairs of settings to find which pairs interact
    let all_settings = vec![
        ConfigOp::SetQuality(85),
        ConfigOp::SetSmoothing(50),
        ConfigOp::SetScanMode(ScanMode::Auto),
        ConfigOp::SetSubsampling422,
        ConfigOp::SetSubsampling444,
        ConfigOp::SetSubsampling420,
        ConfigOp::SetProgressive,
        ConfigOp::SetOptimizeCoding(true),
        ConfigOp::SetOptimizeScans(true),
        ConfigOp::SetUseScansInTrellis(true),
    ];

    let width = 64;
    let height = 64;
    let pixels = create_test_pattern(width, height);

    println!("\n=== Pairwise Interaction Analysis ===\n");
    println!("Testing which pairs of settings have order-dependent behavior:\n");

    let mut interactions = Vec::new();

    for i in 0..all_settings.len() {
        for j in (i + 1)..all_settings.len() {
            let setting_a = all_settings[i];
            let setting_b = all_settings[j];

            // Try both orders
            let order1 = vec![setting_a, setting_b];
            let order2 = vec![setting_b, setting_a];

            let output1 = encode_with_order(&order1, &pixels, width, height);
            let output2 = encode_with_order(&order2, &pixels, width, height);

            if output1 != output2 {
                interactions.push((setting_a, setting_b, output1.len(), output2.len()));
                println!(
                    "⚠️  {} ↔ {} : ORDER MATTERS ({}B vs {}B)",
                    setting_a.name(),
                    setting_b.name(),
                    output1.len(),
                    output2.len()
                );
            } else {
                println!(
                    "✓  {} ↔ {} : order doesn't matter",
                    setting_a.name(),
                    setting_b.name()
                );
            }
        }
    }

    println!("\n=== Summary ===");
    println!(
        "Found {} pairwise interactions where order matters:",
        interactions.len()
    );
    for (a, b, len1, len2) in &interactions {
        println!(
            "  - {} before {} = {}B, reverse = {}B",
            a.name(),
            b.name(),
            len1,
            len2
        );
    }
}

#[test]
fn test_scan_mode_resets_analysis() {
    // Specifically test what gets reset by set_scan_optimization_mode
    let settings_to_test = vec![
        ConfigOp::SetSmoothing(50),
        ConfigOp::SetSubsampling422,
        ConfigOp::SetSubsampling444,
        ConfigOp::SetSubsampling420,
        ConfigOp::SetProgressive,
        ConfigOp::SetOptimizeCoding(true),
        ConfigOp::SetOptimizeScans(true),
        ConfigOp::SetUseScansInTrellis(true),
        ConfigOp::SetRawDataIn(true),
        ConfigOp::SetPixelDensity,
    ];

    let width = 64;
    let height = 64;
    let pixels = create_test_pattern(width, height);

    println!("\n=== Analyzing what set_scan_optimization_mode() resets ===\n");

    for setting in &settings_to_test {
        // Test: setting BEFORE scan_mode vs AFTER scan_mode
        let before = vec![*setting, ConfigOp::SetScanMode(ScanMode::Auto)];
        let after = vec![ConfigOp::SetScanMode(ScanMode::Auto), *setting];

        // Some settings (like raw_data_in) make the encoder incompatible with scanline encoding
        // We catch those and report them separately
        let output_before = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            encode_with_order(&before, &pixels, width, height)
        }))
        .ok();

        let output_after = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            encode_with_order(&after, &pixels, width, height)
        }))
        .ok();

        match (output_before, output_after) {
            (Some(before_output), Some(after_output)) => {
                if before_output != after_output {
                    println!(
                        "⚠️  {} is RESET by set_scan_optimization_mode()",
                        setting.name()
                    );
                    println!(
                        "     Before: {}B, After: {}B (difference: {} bytes)",
                        before_output.len(),
                        after_output.len(),
                        (before_output.len() as i32 - after_output.len() as i32).abs()
                    );
                } else {
                    println!(
                        "✓  {} is PRESERVED by set_scan_optimization_mode()",
                        setting.name()
                    );
                }
            }
            (None, Some(_)) => {
                println!(
                    "⚠️  {} is RESET by set_scan_optimization_mode() (setting before causes error)",
                    setting.name()
                );
            }
            (Some(_), None) => {
                println!("⚠️  {} causes error when set AFTER scan_mode (incompatible with scanline mode)", setting.name());
            }
            (None, None) => {
                println!(
                    "⚠️  {} is incompatible with scanline encoding (error both before and after)",
                    setting.name()
                );
            }
        }
    }
}

fn factorial(n: usize) -> usize {
    (1..=n).product()
}

#[test]
fn test_components_mut_gets_reset() {
    // Specifically test that components_mut() changes get reset by set_scan_optimization_mode
    use mozjpeg::{ColorSpace, Compress, ScanMode};

    let width = 64;
    let height = 64;
    let pixels = create_test_pattern(width, height);

    println!("\n=== Testing components_mut() reset by set_scan_optimization_mode() ===\n");

    // Test 1: Modify components BEFORE set_scan_optimization_mode (WRONG - gets reset)
    let mut comp1 = Compress::new(ColorSpace::JCS_RGB);
    comp1.set_size(width, height);
    comp1.set_color_space(ColorSpace::JCS_YCbCr);
    comp1.set_quality(85.0);

    // Use components_mut() to set 4:2:2 subsampling
    {
        let comps = comp1.components_mut();
        comps[0].h_samp_factor = 2;
        comps[0].v_samp_factor = 1; // 4:2:2
        comps[1].h_samp_factor = 1;
        comps[1].v_samp_factor = 1;
        comps[2].h_samp_factor = 1;
        comps[2].v_samp_factor = 1;
    }

    println!("Before set_scan_optimization_mode():");
    let c = comp1.components();
    println!("  Y component: h={}, v={}", c[0].h_samp_factor, c[0].v_samp_factor);

    comp1.set_scan_optimization_mode(ScanMode::Auto);

    println!("After set_scan_optimization_mode():");
    let c = comp1.components();
    println!("  Y component: h={}, v={} (RESET!)", c[0].h_samp_factor, c[0].v_samp_factor);

    let mut started1 = comp1.start_compress(Vec::new()).unwrap();
    started1.write_scanlines(&pixels).unwrap();
    let jpeg1 = started1.finish().unwrap();

    // Test 2: Modify components AFTER set_scan_optimization_mode (CORRECT - preserved)
    let mut comp2 = Compress::new(ColorSpace::JCS_RGB);
    comp2.set_size(width, height);
    comp2.set_color_space(ColorSpace::JCS_YCbCr);
    comp2.set_quality(85.0);

    comp2.set_scan_optimization_mode(ScanMode::Auto);

    // Use components_mut() AFTER scan_mode
    {
        let comps = comp2.components_mut();
        comps[0].h_samp_factor = 2;
        comps[0].v_samp_factor = 1; // 4:2:2
        comps[1].h_samp_factor = 1;
        comps[1].v_samp_factor = 1;
        comps[2].h_samp_factor = 1;
        comps[2].v_samp_factor = 1;
    }

    println!("\nWith correct ordering (components_mut AFTER scan_mode):");
    let c = comp2.components();
    println!("  Y component: h={}, v={} (PRESERVED!)", c[0].h_samp_factor, c[0].v_samp_factor);

    let mut started2 = comp2.start_compress(Vec::new()).unwrap();
    started2.write_scanlines(&pixels).unwrap();
    let jpeg2 = started2.finish().unwrap();

    // Test 3: No subsampling change (default 4:2:0)
    let mut comp3 = Compress::new(ColorSpace::JCS_RGB);
    comp3.set_size(width, height);
    comp3.set_color_space(ColorSpace::JCS_YCbCr);
    comp3.set_quality(85.0);
    comp3.set_scan_optimization_mode(ScanMode::Auto);

    let mut started3 = comp3.start_compress(Vec::new()).unwrap();
    started3.write_scanlines(&pixels).unwrap();
    let jpeg3 = started3.finish().unwrap();

    println!("\n=== Results ===");
    println!("components_mut BEFORE scan_mode: {} bytes", jpeg1.len());
    println!("components_mut AFTER scan_mode:  {} bytes", jpeg2.len());
    println!("No components_mut (default):     {} bytes", jpeg3.len());

    // The bug: jpeg1 should match jpeg2 (both request 4:2:2)
    // but actually matches jpeg3 (default 4:2:0) because it was reset!
    if jpeg1 == jpeg3 {
        println!("\n⚠️  BUG CONFIRMED: components_mut() changes are RESET by set_scan_optimization_mode()!");
        println!("Setting components BEFORE scan_mode has no effect - they revert to default.");
    }

    if jpeg2 != jpeg3 {
        println!("✓  Correct ordering (components_mut AFTER scan_mode) produces different output");
    }

    assert_ne!(
        jpeg1, jpeg2,
        "BUG: Order of components_mut() vs set_scan_optimization_mode() affects output"
    );
}

#[test]
fn test_comprehensive_5_settings() {
    // Test 5 settings = 120 permutations
    let settings = vec![
        ConfigOp::SetQuality(85),
        ConfigOp::SetSmoothing(50),
        ConfigOp::SetScanMode(ScanMode::Auto),
        ConfigOp::SetSubsampling422,
        ConfigOp::SetProgressive,
    ];

    let width = 64;
    let height = 64;
    let pixels = create_test_pattern(width, height);

    println!(
        "\n=== Comprehensive test: {} settings ({} permutations) ===",
        settings.len(),
        factorial(settings.len())
    );

    let perms = permutations(&settings);
    let mut output_groups: HashMap<Vec<u8>, Vec<Vec<ConfigOp>>> = HashMap::new();

    for perm in perms {
        let output = encode_with_order(&perm, &pixels, width, height);
        output_groups.entry(output).or_default().push(perm);
    }

    println!(
        "Found {} unique outputs from {} orderings\n",
        output_groups.len(),
        factorial(settings.len())
    );

    // Analyze patterns in the groups
    for (i, (output, orderings)) in output_groups.iter().enumerate() {
        println!(
            "Output variant {} ({} bytes, {} orderings):",
            i + 1,
            output.len(),
            orderings.len()
        );

        // Find common patterns in this group
        // Count how often scan_mode appears at each position
        let mut scan_mode_positions: HashMap<usize, usize> = HashMap::new();
        for ordering in orderings {
            if let Some(pos) = ordering
                .iter()
                .position(|op| matches!(op, ConfigOp::SetScanMode(_)))
            {
                *scan_mode_positions.entry(pos).or_insert(0) += 1;
            }
        }

        println!("  scan_mode position distribution:");
        for pos in 0..settings.len() {
            if let Some(count) = scan_mode_positions.get(&pos) {
                println!("    Position {}: {} orderings", pos, count);
            }
        }

        println!(
            "  Example ordering: {}",
            orderings[0]
                .iter()
                .map(|op| op.name())
                .collect::<Vec<_>>()
                .join(" → ")
        );
        println!();
    }

    if output_groups.len() > 1 {
        println!(
            "⚠️  ORDERING MATTERS! Found {} different outputs",
            output_groups.len()
        );
    }
}

#[test]
fn test_all_subsampling_modes() {
    // Test all 3 subsampling modes with scan_mode
    let settings = vec![
        ConfigOp::SetSubsampling420,
        ConfigOp::SetSubsampling422,
        ConfigOp::SetSubsampling444,
        ConfigOp::SetScanMode(ScanMode::Auto),
    ];

    let width = 64;
    let height = 64;
    let pixels = create_test_pattern(width, height);

    println!(
        "\n=== Testing all subsampling modes ({} permutations) ===",
        factorial(settings.len())
    );

    let perms = permutations(&settings);
    let mut output_groups: HashMap<Vec<u8>, Vec<Vec<ConfigOp>>> = HashMap::new();

    for perm in perms {
        let output = encode_with_order(&perm, &pixels, width, height);
        output_groups.entry(output).or_default().push(perm);
    }

    println!("Found {} unique outputs\n", output_groups.len());

    // Group by which subsampling mode was used last (determines final output)
    let mut final_subsampling: HashMap<String, Vec<Vec<ConfigOp>>> = HashMap::new();

    for (_output, orderings) in output_groups.iter() {
        for ordering in orderings {
            // Find the last subsampling operation
            let last_subsampling = ordering
                .iter()
                .rev()
                .find(|op| {
                    matches!(
                        op,
                        ConfigOp::SetSubsampling420
                            | ConfigOp::SetSubsampling422
                            | ConfigOp::SetSubsampling444
                    )
                })
                .map(|op| op.name())
                .unwrap_or("none".to_string());

            final_subsampling
                .entry(last_subsampling)
                .or_default()
                .push(ordering.clone());
        }
    }

    println!("Grouped by final subsampling setting:");
    for (subsampling, orderings) in final_subsampling {
        println!(
            "  {} (final): {} orderings produce this",
            subsampling,
            orderings.len()
        );
    }
}

#[test]
fn test_scan_mode_variants() {
    // Test different scan mode values
    let scan_modes = vec![
        ScanMode::AllComponentsTogether,
        ScanMode::ScanPerComponent,
        ScanMode::Auto,
    ];

    let width = 64;
    let height = 64;
    let pixels = create_test_pattern(width, height);

    println!("\n=== Testing different ScanMode variants ===\n");

    for mode in &scan_modes {
        let settings = vec![
            ConfigOp::SetSmoothing(50),
            ConfigOp::SetScanMode(*mode),
            ConfigOp::SetSubsampling422,
        ];

        let perms = permutations(&settings);
        let mut output_groups: HashMap<Vec<u8>, usize> = HashMap::new();

        for perm in perms {
            let output = encode_with_order(&perm, &pixels, width, height);
            *output_groups.entry(output).or_insert(0) += 1;
        }

        println!(
            "ScanMode::{:?}: {} unique outputs from {} orderings",
            mode,
            output_groups.len(),
            factorial(settings.len())
        );
    }
}
