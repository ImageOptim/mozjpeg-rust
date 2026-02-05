# Changelog

## [0.11.0] - 2026-02-04

### Breaking Changes

- **Lazy configuration**: All settings are now collected and applied at `start_compress()` time instead of immediately. This fixes configuration ordering bugs but changes semantics:
  - All setter methods store to a pending configuration
  - Settings are applied in the correct order when `start_compress()` is called
  - `components()` and `cinfo()` now take `&mut self` instead of `&self`, since they apply pending config before returning so callers always see up-to-date values

- **Deprecated `components_mut()` and `cinfo_mut()`**: Use `mutate_components_last()` and `mutate_cinfo_last()` instead for order-independent configuration.

- **Removed `set_quality_force_8bit()`, `set_luma_qtable_force_8bit()`, `set_chroma_qtable_force_8bit()`** (added in #50, never released): With lazy configuration, 8-bit quantization control is a single independent setting via `set_force_8bit_quantization()`. The `_force_8bit` method variants are unnecessary since all settings are buffered and applied together.

### Fixed

- Configuration ordering bugs where `set_scan_optimization_mode()` and `set_fastest_defaults()` would reset other settings via internal `jpeg_set_defaults()` calls. Settings like quality, smoothing, subsampling, pixel density, and progressive mode are now preserved regardless of call order.

### Added

- `mutate_components_last()` - Modify components via a callback that runs after all other configuration
- `mutate_cinfo_last()` - Access raw `cinfo` via a callback that runs after all other configuration
- `set_force_8bit_quantization()` - Control 8-bit DQT clamping (applies to both quality and custom qtables)
- `write_scanlines_strided()` - Write scanlines with custom stride for padded pixel data
- `Copy`, `Clone`, `Debug`, `PartialEq`, `Eq` derives for `PixelDensity` and `PixelDensityUnit`
- Validation in `start_compress()` returns `io::Error` instead of panicking for:
  - Zero dimensions
  - Dimensions exceeding JPEG maximum (65535)
  - Invalid sampling factors (≤0 or >4)

### Migration Guide

Before 0.11.0, the order of configuration calls mattered:

```rust
// BROKEN in 0.10.x: smoothing gets reset to 0!
comp.set_smoothing_factor(50);
comp.set_scan_optimization_mode(ScanMode::Auto);
```

In 0.11.0, order no longer matters:

```rust
// Both orderings produce identical results in 0.11.0
comp.set_smoothing_factor(50);
comp.set_scan_optimization_mode(ScanMode::Auto);

// Same result as:
comp.set_scan_optimization_mode(ScanMode::Auto);
comp.set_smoothing_factor(50);
```

**Migrating from `components_mut()`**: Use `mutate_components_last()` instead:

```rust
// Before (0.10.x):
comp.components_mut()[0].h_samp_factor = 2;

// After (0.11.0):
comp.mutate_components_last(|components| {
    components[0].h_samp_factor = 2;
});
```

## [0.10.13] and earlier

See git history for previous changes.
