# Rust wrapper for MozJPEG library

Convenient reading and writing of well-compressed JPEG images using a safe Rust interface.

The interface is still being developed, so it has rough edges and may change.

## Decoding

```rust
let d = mozjpeg::Decompress::with_markers(mozjpeg::ALL_MARKERS)
    .from_path("tests/test.jpg")?;

d.width();
d.height();
d.color_space() == mozjpeg::ColorSpace::JCS_YCbCr;
for marker in d.markers() {}

let image = d.rgb().unwrap();
image.width();
image.height();
image.color_space() == mozjpeg::ColorSpace::JCS_RGB;
```

## Error handling

The libjpeg library will in some cases `panic!()` on error. You can use `catch_panic()` to handle these errors.
