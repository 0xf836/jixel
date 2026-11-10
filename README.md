# jixel

A tiny JPEG XL encoder in Rust, ported and reworked from
[libjxl/libjxl-tiny](https://github.com/libjxl/libjxl-tiny).

## Command-line encoder

```sh
cargo build --release -p jixel-cli
cargo install --path cli
jixel image.png image.jxl -q 90
jixel photo.jpg photo.jxl              # reversible JPEG transcoding
jixel image.png exact.jxl --lossless
jixel --help
```

The CLI supports PNG/JPEG input, 8/16-bit grayscale and alpha, progressive
encoding, color management, and encoder presets. See the [CLI guide](cli/README.md)
for modes, metadata handling, advanced settings, and limitations.

## Example

```rust
fn main() {
    let output = "encoded_lossy2.jxl";
    let image = image::open(Path::new("./assets/abstract_alpha.png")).unwrap();
    let bytes = jixel::encode_image_with_alpha(
        image.to_rgba8().as_raw(),
        image.width() as usize,
        image.height() as usize,
        &EncodeConfig::default().with_quality(90.),
    );
    std::fs::write(&output, &bytes).expect("failed to write output");
}
```

## License

This project is licensed under either of

- BSD-3-Clause License (see [LICENSE](LICENSE.md))
- Apache License, Version 2.0 (see [LICENSE](LICENSE-APACHE.md))

at your option.
