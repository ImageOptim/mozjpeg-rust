# Rust wrapper for MozJPEG library

This library is compatible with Rust versions 1.27 to 1.32 only. In Rust 1.33 or later any errors will cause the entire process to abort due to changes in Rust's unwinding

----

This library offers convenient reading and writing of well-compressed JPEG images using a safe Rust interface.

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

In Rust v1.32 and older, errors detected by libjpeg cause `panic!()`, and you can use `catch_unwind()` to handle these errors gracefully.

In Rust v1.33 and later ([until issue #58760 is resolved](https://github.com/rust-lang/rust/issues/58760)) any error in libjpeg causes a crash of the entire process, and there is no way to gracefully handle even most trivial errors.
