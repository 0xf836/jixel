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

For a single file, both input and output are required. Formats are detected by
content. PNG defaults to lossy distance 1; JPEG defaults to coefficient
transcoding with original-byte reconstruction. `--jpeg` selects that mode
explicitly. `--no-jpeg-reconstruction` omits reconstruction data while preserving
the coefficients. Unsupported JPEGs
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

## Batch processing

A directory input enables batch processing. Specify an output directory, or omit
it to write JXL files beside their sources:

```sh
jixel photos encoded -q 90
jixel photos -q 90
jixel photos encoded --recursive --min-long-edge 1600 -q 90
jixel photos encoded --name '{stem}-{ext}.jxl' -q 90
jixel photos encoded --jobs 4 --threads 16 -q 90
```

Without `--recursive`, only immediate files are considered. Recursion preserves
relative subdirectories; an output directory inside the input is excluded from
the scan. Symbolic links are skipped. PNG and JPEG are detected by content,
regardless of extension; other files are skipped. Inputs are collected before
encoding starts. An empty or entirely skipped batch succeeds.

`--min-long-edge N` skips images whose longest side is **less than N pixels**;
equality passes. Dimensions come from headers without decoding pixels. The
threshold does not resize images. The same quality, speed, and metadata options
apply to every input, including the usual format-specific mode restrictions.
JPEG still defaults to reversible transcoding, so select `-q` or `-d` explicitly
when reducing JPEG quality is intended.

`--name TEMPLATE` defaults to `{stem}.jxl`. Only `{stem}` (filename without its
last extension) and `{ext}` (extension without its dot) are supported, preserving
case and native filename bytes. Templates must be single filenames ending in
`.jxl`, without path separators. For example, `{stem}-{ext}.jxl` distinguishes
`photo.jpg` from `photo.png`. Colliding output names and outputs referring to
any batch input are rejected before writing, including with `--force`. Name
collisions follow the destination filesystem's case and Unicode equivalence
rules. For new outputs, temporary name probes in existing destination ancestors
are removed before encoding starts.

Existing regular output files are skipped; `--force` atomically replaces them.
Directories and other non-file objects at output paths are errors. File errors,
including blocked output subdirectories, are reported while other inputs
continue. The final summary counts successful, skipped, and failed items
(including skipped non-images). Exit status is 0 for
success, 1 for processing errors, and 2 for invalid arguments. `--quiet` hides
success/skip messages and the summary, while errors remain visible. Batch options
require directory input; batch output cannot be stdout.

`--jobs N` limits concurrent images (default: up to four, capped by the CPU budget
and input count). In batch mode, `-j/--threads N` is the **total encoder thread
budget**, divided between workers, rather than a budget for each image. Zero
uses available CPUs, as in single-file mode. More jobs overlap serial decoding,
color preparation, and I/O with encoding, but increase memory use: each active
image holds its own buffers and encoder state. The queue holds paths, not decoded
images. Reduce `--jobs` for large images or limited memory. A fixed budget per
worker can leave CPUs idle near the end of a batch; continuous full CPU use is
not guaranteed.

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
