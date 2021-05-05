# Rust wrapper for MozJPEG library

This library adds a safe(r) interface on top of libjpeg-turbo and MozJPEG for reading and writing well-compressed JPEG images.

The interface is still being developed, so it has rough edges and may change.

In particular, error handling is weird due to libjpeg's peculiar design. Error handling can't use `Result`, but needs to depend on Rust's `resume_unwind` (a panic, basically) to signal any errors in libjpeg. It's necessary to wrap all uses of this library in `catch_unwind`.

In crates compiled with `panic=abort` setting, any JPEG error will abort the process.

## Decoding example

```rust
std::panic::catch_unwind(|| {
    let d = mozjpeg::Decompress::with_markers(mozjpeg::ALL_MARKERS)
        .from_path("tests/test.jpg")?;

    d.width(); // FYI
    d.height();
    d.color_space() == mozjpeg::ColorSpace::JCS_YCbCr;
    for marker in d.markers() {}

    // rgb() enables conversiono
    let image = d.rgb()?;
    image.width();
    image.height();
    image.color_space() == mozjpeg::ColorSpace::JCS_RGB;

    let pixels = image.read_scanlines()?;
    assert!(image.finish_decompress());
    Ok(())
})?;
```

## Encoding example

```rust
std::panic::catch_unwind(|| {
    let mut comp = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);

    comp.set_size(width, height);
    comp.set_mem_dest();
    comp.start_compress();

    // replace with your image data
    let pixels = vec![0; width * height * 3];
    assert!(comp.write_scanlines(&pixels[..]));

    comp.finish_compress();
    let jpeg_bytes = comp.data_to_vec()?;
    // write to file, etc.
    Ok(())
})?;
```
