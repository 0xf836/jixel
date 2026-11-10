# jixel CLI

Encode PNG and JPEG images to JPEG XL using the local `jixel` library.

```sh
cargo build --release -p jixel-cli
cargo install --path cli
jixel --help
```

## Modes and usage

```sh
jixel image.png image.jxl -q 90
jixel photo.jpg photo.jxl
jixel photo.jpg smaller.jxl -d 1 -s slow
jixel rgba.png exact.jxl --lossless
jixel image.png preview.jxl --progressive --group-order center-first
cat image.png | jixel - - -q 90 > image.jxl
```

Both input and output are required. Formats are detected by content. PNG defaults
to lossy distance 1; JPEG defaults to coefficient transcoding with original-byte
reconstruction. `--jpeg` selects that mode explicitly. `--no-jpeg-reconstruction`
omits reconstruction data while preserving the coefficients. Unsupported JPEGs
fail rather than falling back to pixel encoding.

`-q/--quality` (0..100), `-d/--distance` (0.05..25), `--lossless`, and `--jpeg`
are mutually exclusive. **Quality 100 is still lossy.** `--lossless` preserves
decoded samples, including alpha and invisible colors; for JPEG input this does
not preserve the original JPEG bytes. `--near-lossless N --lossless` permits at
most N code values of error per color sample, while alpha stays exact.

`-s/--speed` accepts `ultrafast`, `fastest`, `fast` (default), `medium`,
`medium-slow`, `slow`, and `extra-slow`. Slower presets spend more time searching
for denser output. `-j/--threads 0` (default) uses available cores. Explicit tool
switches override the preset regardless of argument order; for example,
`-s slow --no-patches --no-vector-layers` disables those preset tools.

## Formats, color, and metadata

PNG supports grayscale, grayscale+alpha, RGB, and RGBA at 8/16 bits. Palette,
transparency (`tRNS`), and packed grayscale inputs are expanded without reducing
precision. APNG is rejected. JPEG can be transcoded or decoded for pixel encoding.

`--color-encoding` overrides the **source** interpretation with a named space;
`--icc FILE` supplies a source profile. These options are mutually exclusive.
Named spaces are `srgb`, `linear-srgb`, `display-p3`, `bt2020`, `bt2100-pq`, and
`bt2100-hlg`. Named spaces are passed to the library directly. HDR remains HDR;
there is no implicit tone mapping. Peak defaults are 10000 nits for PQ, 1000 for
HLG, and the library's 255-nit SDR default. Override the peak with
`--intensity-target`.

EXIF/XMP are preserved by default. `--exif FILE` replaces EXIF with raw TIFF bytes;
`--xmp FILE` replaces the UTF-8 XMP packet. Orientation is carried in the JXL
codestream without rotating stored pixels. `--strip-metadata` removes EXIF/XMP
while retaining color and orientation. Other source metadata is not guaranteed
to survive pixel encoding; embedded thumbnails and descriptive tags are copied,
not regenerated to describe changed pixels.

JPEG transcoding uses the library's ICC, EXIF/XMP, orientation, and Ultra HDR
handling. Binary ISO gain maps survive with or without JPEG reconstruction;
XMP-only gain maps are retained in reconstruction data. Pixel encoding does not
preserve a source JPEG gain map. `--strip-metadata` and source overrides are
rejected in transcoding mode: its reconstruction data includes original metadata
and the API does not expose metadata removal. Select a pixel mode to strip EXIF/XMP.

## Advanced options and API coverage

`-h` gives a short reference; `--help` includes all options, accepted values, units,
defaults, and tool restrictions. 

| Library capability | CLI interface |
| --- | --- |
| Quality, distance, all seven speeds, threads | `-q`, `-d`, `-s`, `-j` |
| Lossless, near-lossless, decoding effort, exact cell upsampling | `--lossless`, `--near-lossless`, `--decoding-speed`, `--upsampling` |
| Alpha distance and invisible colors | `--alpha-distance`, `--[no-]fill-invisible` |
| Progressive lossless/lossy and VarDCT pass schedule | `--progressive`, `--progressive-passes`, `--progressive-shifts` |
| DC previews and group order | `--progressive-dc`, `--progressive-dc-levels`, `--group-order` |
| Coding transform, modular selection, perceptual metric, learned rate | `--coding`, `--lossy-modular`, `--arbiter`, `--[no-]learned-rate` |
| Preset and experimental content tools | `--[no-]patches`, `--[no-]splines`, `--[no-]edge-splines`, `--[no-]dots`, `--[no-]ink-layer`, `--[no-]vector-layers` |
| All grain variants | `--grain off\|auto`, `--photon-noise-iso`, `--measured-grain`, `--grain-lut` |
| Source color, ICC, HDR, EXIF/XMP, orientation | Automatic input metadata and the overrides described above |
| JPEG coefficient coding, effort, reconstruction, Ultra HDR | JPEG mode, `-s`, `-j`, `--no-jpeg-reconstruction` |
| Raw 10/12-bit, float/CMYK entry points, fast-lossless backend | Library only |
| Manual gain-map construction and custom Brotli callbacks | Library only |

## Development checks

```sh
cargo test -p jixel-cli
cargo clippy -p jixel-cli --all-targets -- -D warnings
cargo fmt -p jixel-cli -- --check
cargo test -p jixel-cli -- --include-ignored
```
