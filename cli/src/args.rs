use clap::{CommandFactory, Parser, ValueEnum, error::ErrorKind};
use jixel::{EncodeConfig, Speed};
use std::path::PathBuf;

const EXAMPLES: &str = "EXAMPLES:
  jixel photo.png photo.jxl -q 90
  jixel photo.jpg photo.jxl                       # reversible JPEG transcode
  jixel photo.jpg smaller.jxl -q 85 -s slow        # decode and encode pixels
  jixel image.png exact.jxl --lossless
  jixel image.png preview.jxl --progressive -d 1
  jixel photos encoded --recursive -q 90
  jixel photos --min-long-edge 1600 --jobs 4 -q 90
  cat image.png | jixel - - -q 90 > image.jxl

JPEG input defaults to reversible transcoding. PNG defaults to lossy distance 1.
Quality 100 is still lossy: use --lossless for exact pixels, or JPEG transcoding
to recover the original JPEG bytes. Use --help for all options and limitations.";

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum SpeedArg {
    Ultrafast,
    Fastest,
    Fast,
    Medium,
    MediumSlow,
    Slow,
    ExtraSlow,
}

impl From<SpeedArg> for Speed {
    fn from(value: SpeedArg) -> Self {
        match value {
            SpeedArg::Ultrafast => Self::UltraFast,
            SpeedArg::Fastest => Self::Fastest,
            SpeedArg::Fast => Self::Fast,
            SpeedArg::Medium => Self::Medium,
            SpeedArg::MediumSlow => Self::MediumSlow,
            SpeedArg::Slow => Self::Slow,
            SpeedArg::ExtraSlow => Self::ExtraSlow,
        }
    }
}

#[derive(Clone, Debug, Parser)]
#[command(name = "jixel", version, about = "Encode PNG and JPEG images to JPEG XL",
    long_about = "Encode PNG and JPEG images to JPEG XL.\n\nPNG supports grayscale, RGB and alpha at 8/16 bits, including palette and packed\ninputs. JPEG defaults to reversible coefficient transcoding; -q/-d or --lossless\nselect pixel encoding instead. SDR ICC input is converted to sRGB only in lossy\nmode. Lossless retains samples and ICC. Supported PQ/HLG PNG stays HDR.\n\nInput and encoded output are buffered in memory; this is not a streaming encoder.",
    after_help = EXAMPLES, max_term_width = 100, next_line_help = false,
    arg_required_else_help = true)]
pub struct Args {
    /// PNG/JPEG file or input directory; '-' reads buffered stdin
    pub input: PathBuf,
    /// Output file/directory; omitted for a directory writes beside inputs
    pub output: Option<PathBuf>,

    /// Include subdirectories, preserving their structure
    #[arg(long, help_heading = "Batch")]
    pub recursive: bool,
    /// Maximum concurrent images [default for directories: up to 4]
    #[arg(long, value_parser = positive_usize, value_name = "N", help_heading = "Batch")]
    pub jobs: Option<usize>,
    /// Skip images whose longest side is less than N pixels
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..), value_name = "N", help_heading = "Batch")]
    pub min_long_edge: Option<u32>,
    /// Output filename using {stem} and {ext} [default: {stem}.jxl]
    #[arg(long, value_name = "TEMPLATE", help_heading = "Batch")]
    pub name: Option<String>,

    /// Lossy quality, 0..100 (100 is still lossy) [default: distance 1]
    #[arg(short = 'q', long, value_parser = quality, conflicts_with_all = ["distance", "lossless", "jpeg"], help_heading = "Quality & modes")]
    pub quality: Option<f32>,
    /// Lossy distance, 0.05..25; lower is better quality
    #[arg(short = 'd', long, value_parser = distance, conflicts_with_all = ["lossless", "jpeg"], help_heading = "Quality & modes")]
    pub distance: Option<f32>,
    /// Preserve decoded pixels exactly (not original JPEG bytes)
    #[arg(long, conflicts_with = "jpeg", help_heading = "Quality & modes")]
    pub lossless: bool,
    /// Transcode JPEG coefficients without requantization [default for JPEG]
    #[arg(long, help_heading = "Quality & modes")]
    pub jpeg: bool,
    /// Omit original-JPEG reconstruction data; coefficients stay unchanged
    #[arg(long, help_heading = "Quality & modes")]
    pub no_jpeg_reconstruction: bool,

    /// Encoder effort: ultrafast to extra-slow, faster to denser
    #[arg(
        short = 's',
        long,
        value_enum,
        default_value = "fast",
        help_heading = "Performance"
    )]
    pub speed: SpeedArg,
    /// Worker threads (total budget for directories); 0 uses available CPUs
    #[arg(short = 'j', long, default_value_t = 0, help_heading = "Performance")]
    #[arg(value_name = "N")]
    pub threads: usize,

    /// Maximum sample error; requires --lossless; alpha stays exact
    #[arg(
        long,
        requires = "lossless",
        hide_short_help = true,
        help_heading = "Lossless & alpha"
    )]
    #[arg(value_name = "N")]
    pub near_lossless: Option<u16>,
    /// Decode speed/density tradeoff for lossless and lossy alpha
    #[arg(long, value_parser = ["slow", "fast", "fastest"], hide_short_help = true, help_heading = "Lossless & alpha")]
    #[arg(value_name = "SPEED")]
    pub decoding_speed: Option<String>,
    /// Detect exact nearest-neighbor cell grids (lossless, non-progressive)
    #[arg(
        long,
        requires = "lossless",
        conflicts_with = "progressive",
        hide_short_help = true,
        help_heading = "Lossless & alpha"
    )]
    pub upsampling: bool,
    /// Lossy alpha distance, 0..25; 0 keeps alpha exact [default: frame distance]
    #[arg(long, value_parser = alpha_distance, hide_short_help = true, help_heading = "Lossless & alpha")]
    #[arg(value_name = "DISTANCE")]
    pub alpha_distance: Option<f32>,
    /// Fill invisible colors before lossy encoding [default: enabled]
    #[arg(
        long,
        conflicts_with = "no_fill_invisible",
        hide_short_help = true,
        help_heading = "Lossless & alpha"
    )]
    pub fill_invisible: bool,
    /// Preserve invisible colors during lossy preprocessing
    #[arg(long, hide_short_help = true, help_heading = "Lossless & alpha")]
    pub no_fill_invisible: bool,

    /// Progressive lossless Squeeze, or two-pass lossy VarDCT
    #[arg(long, conflicts_with_all = ["progressive_passes", "progressive_shifts"], hide_short_help = true, help_heading = "Progressive")]
    pub progressive: bool,
    /// Lossy VarDCT pass count, 1..4 (conflicts with explicit shifts)
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=4), conflicts_with = "progressive_shifts", hide_short_help = true, help_heading = "Progressive")]
    #[arg(value_name = "N")]
    pub progressive_passes: Option<u32>,
    /// Lossy pass shifts, e.g. 2,1,0; descending 0..3, ending in 0
    #[arg(
        long,
        value_delimiter = ',',
        num_args = 1,
        hide_short_help = true,
        help_heading = "Progressive"
    )]
    #[arg(value_name = "SHIFTS")]
    pub progressive_shifts: Option<Vec<u32>>,
    /// Progressive DC preview pyramid (lossy XYB RGB without alpha only)
    #[arg(long, hide_short_help = true, help_heading = "Progressive")]
    pub progressive_dc: bool,
    /// DC preview squeeze levels, >=1; implies --progressive-dc
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..), hide_short_help = true, help_heading = "Progressive")]
    #[arg(value_name = "N")]
    pub progressive_dc_levels: Option<u32>,
    /// Lossy VarDCT group order [default: scanline]
    #[arg(long, value_parser = ["scanline", "center-first", "diagonals"], hide_short_help = true, help_heading = "Progressive")]
    #[arg(value_name = "ORDER")]
    pub group_order: Option<String>,

    /// Lossy color transform [default: xyb]
    #[arg(long, value_parser = ["xyb", "ycbcr", "rgb"], hide_short_help = true, help_heading = "Encoder tools")]
    #[arg(value_name = "TRANSFORM")]
    pub coding: Option<String>,
    /// Lossy modular selection; auto requires slow/extra-slow [default: off]
    #[arg(long, value_parser = ["off", "auto", "force"], hide_short_help = true, help_heading = "Encoder tools")]
    #[arg(value_name = "MODE")]
    pub lossy_modular: Option<String>,
    /// Perceptual metric for lossy decisions [default: ssimulacra2]
    #[arg(long, value_parser = ["ssimulacra2", "cvvdp"], hide_short_help = true, help_heading = "Encoder tools")]
    #[arg(value_name = "METRIC")]
    pub arbiter: Option<String>,
    /// Enable learned rate prices (medium-slow and above; default on)
    #[arg(
        long,
        conflicts_with = "no_learned_rate",
        hide_short_help = true,
        help_heading = "Encoder tools"
    )]
    pub learned_rate: bool,
    /// Disable learned rate prices
    #[arg(long, hide_short_help = true, help_heading = "Encoder tools")]
    pub no_learned_rate: bool,
    /// Enable patch dictionary [default: speed preset]
    #[arg(
        long,
        conflicts_with = "no_patches",
        hide_short_help = true,
        help_heading = "Encoder tools"
    )]
    pub patches: bool,
    /// Disable patch dictionary, including speed-preset patches
    #[arg(long, hide_short_help = true, help_heading = "Encoder tools")]
    pub no_patches: bool,
    /// Experimental thin-line splines (slow/extra-slow, lossy XYB only)
    #[arg(
        long,
        conflicts_with = "no_splines",
        hide_short_help = true,
        help_heading = "Encoder tools"
    )]
    pub splines: bool,
    /// Disable thin-line splines
    #[arg(long, hide_short_help = true, help_heading = "Encoder tools")]
    pub no_splines: bool,
    /// Experimental edge splines (slow+, XYB, distance >1, single pass)
    #[arg(
        long,
        conflicts_with = "no_edge_splines",
        hide_short_help = true,
        help_heading = "Encoder tools"
    )]
    pub edge_splines: bool,
    /// Disable edge splines
    #[arg(long, hide_short_help = true, help_heading = "Encoder tools")]
    pub no_edge_splines: bool,
    /// Experimental small marks (slow/extra-slow, lossy XYB only)
    #[arg(
        long,
        conflicts_with = "no_dots",
        hide_short_help = true,
        help_heading = "Encoder tools"
    )]
    pub dots: bool,
    /// Disable small-mark coding
    #[arg(long, hide_short_help = true, help_heading = "Encoder tools")]
    pub no_dots: bool,
    /// Experimental text layer (lossy 8-bit RGB/RGBA XYB, no progressive DC)
    #[arg(
        long,
        conflicts_with = "no_ink_layer",
        hide_short_help = true,
        help_heading = "Encoder tools"
    )]
    pub ink_layer: bool,
    /// Disable text layer, including speed-preset layers
    #[arg(long, hide_short_help = true, help_heading = "Encoder tools")]
    pub no_ink_layer: bool,
    /// Experimental flat-art layers (lossy 8-bit RGB only)
    #[arg(
        long,
        conflicts_with = "no_vector_layers",
        hide_short_help = true,
        help_heading = "Encoder tools"
    )]
    pub vector_layers: bool,
    /// Disable flat-art layers, including speed-preset layers
    #[arg(long, hide_short_help = true, help_heading = "Encoder tools")]
    pub no_vector_layers: bool,

    /// Decoder grain synthesis for lossy VarDCT [default: off]
    #[arg(long, value_parser = ["off", "auto"], conflicts_with_all = ["photon_noise_iso", "measured_grain", "grain_lut"], hide_short_help = true, help_heading = "Encoder tools")]
    #[arg(value_name = "MODE")]
    pub grain: Option<String>,
    /// Photon noise at this ISO sensitivity (>0)
    #[arg(long, value_parser = positive, conflicts_with_all = ["measured_grain", "grain_lut"], hide_short_help = true, help_heading = "Encoder tools")]
    #[arg(value_name = "ISO")]
    pub photon_noise_iso: Option<f32>,
    /// Measure, remove and resynthesize source noise at this scale (>0)
    #[arg(long, value_parser = positive, conflicts_with = "grain_lut", hide_short_help = true, help_heading = "Encoder tools")]
    #[arg(value_name = "SCALE")]
    pub measured_grain: Option<f32>,
    /// Eight comma-separated grain strengths in 0..1
    #[arg(long, value_delimiter = ',', num_args = 1, value_parser = unit, hide_short_help = true, help_heading = "Encoder tools")]
    #[arg(value_name = "V1,...,V8")]
    pub grain_lut: Option<Vec<f32>>,

    /// Override SOURCE color interpretation; does not just relabel the output
    #[arg(long, value_parser = ["srgb", "linear-srgb", "display-p3", "bt2020", "bt2100-pq", "bt2100-hlg"], conflicts_with = "icc", hide_short_help = true, help_heading = "Color & HDR")]
    #[arg(value_name = "SPACE")]
    pub color_encoding: Option<String>,
    /// Override source ICC; lossy SDR converts to sRGB, lossless embeds it
    #[arg(
        long,
        value_name = "FILE",
        hide_short_help = true,
        help_heading = "Color & HDR"
    )]
    pub icc: Option<PathBuf>,
    /// Peak luminance in nits (>0) [default: PQ 10000, HLG 1000, SDR 255]
    #[arg(long, value_parser = positive, hide_short_help = true, help_heading = "Color & HDR")]
    #[arg(value_name = "NITS")]
    pub intensity_target: Option<f32>,
    /// Replace EXIF with raw TIFF bytes from FILE
    #[arg(
        long,
        value_name = "FILE",
        conflicts_with = "strip_metadata",
        hide_short_help = true,
        help_heading = "Metadata"
    )]
    pub exif: Option<PathBuf>,
    /// Replace XMP with packet bytes from FILE
    #[arg(
        long,
        value_name = "FILE",
        conflicts_with = "strip_metadata",
        hide_short_help = true,
        help_heading = "Metadata"
    )]
    pub xmp: Option<PathBuf>,
    /// Omit EXIF/XMP, retaining color and orientation (pixel modes only)
    #[arg(long, hide_short_help = true, help_heading = "Metadata")]
    pub strip_metadata: bool,

    /// Atomically replace an existing output (never the input itself)
    #[arg(short = 'f', long, help_heading = "Output")]
    pub force: bool,
    /// Suppress success/skip messages and batch summary; errors stay visible
    #[arg(long, help_heading = "Output")]
    pub quiet: bool,
}

pub fn invalid(message: impl Into<String>) -> clap::Error {
    Args::command().error(ErrorKind::ValueValidation, message.into())
}

fn positive_usize(value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .ok()
        .filter(|&n| n > 0)
        .ok_or_else(|| "expected a positive integer".to_owned())
}

fn number(value: &str, min: f32, max: f32) -> Result<f32, String> {
    let value: f32 = value.parse().map_err(|_| "expected a number".to_owned())?;
    if !value.is_finite() || value < min || value > max {
        return Err(format!("expected a finite number in {min}..={max}"));
    }
    Ok(value)
}
fn quality(v: &str) -> Result<f32, String> {
    number(v, 0., 100.)
}
fn distance(v: &str) -> Result<f32, String> {
    number(v, 0.05, 25.)
}
fn alpha_distance(v: &str) -> Result<f32, String> {
    number(v, 0., 25.)
}
fn positive(v: &str) -> Result<f32, String> {
    number(v, f32::MIN_POSITIVE, f32::MAX)
}
fn unit(v: &str) -> Result<f32, String> {
    number(v, 0., 1.)
}

impl Args {
    pub fn transcode(&self, jpeg_input: bool) -> bool {
        self.jpeg
            || (jpeg_input && self.quality.is_none() && self.distance.is_none() && !self.lossless)
    }

    pub fn config(&self) -> Result<EncodeConfig, clap::Error> {
        let mut c = EncodeConfig::default()
            .with_speed(self.speed.into())
            .with_num_threads(if self.threads == 0 {
                std::thread::available_parallelism().map_or(1, |n| n.get())
            } else {
                self.threads
            })
            .with_lossless(self.lossless);
        if let Some(q) = self.quality {
            c = c.with_quality(q);
        }
        if let Some(d) = self.distance {
            c = c.with_distance(d);
        }
        c.near_lossless = self.near_lossless.unwrap_or(0);
        c.decoding_speed = match self.decoding_speed.as_deref() {
            Some("fast") => jixel::DecodingSpeed::Fast,
            Some("fastest") => jixel::DecodingSpeed::Fastest,
            _ => jixel::DecodingSpeed::Slow,
        };
        c.upsampling = self.upsampling;
        c.alpha_distance = self.alpha_distance;
        c.progressive = self.progressive;
        c.progressive_passes = self.progressive_passes;
        c.progressive_shifts = self.progressive_shifts.clone();
        c.progressive_dc = self.progressive_dc || self.progressive_dc_levels.is_some();
        c.progressive_dc_levels = self.progressive_dc_levels;
        c.group_order = match self.group_order.as_deref() {
            Some("center-first") => jixel::GroupOrder::CenterFirst,
            Some("diagonals") => jixel::GroupOrder::Diagonals,
            _ => jixel::GroupOrder::Scanline,
        };
        c.coding_transform = match self.coding.as_deref() {
            Some("rgb") => jixel::CodingTransform::Rgb,
            Some("ycbcr") => jixel::CodingTransform::YCbCr,
            _ => jixel::CodingTransform::Xyb,
        };
        c.lossy_modular = match self.lossy_modular.as_deref() {
            Some("auto") => jixel::LossyModular::Auto,
            Some("force") => jixel::LossyModular::Force,
            _ => jixel::LossyModular::Off,
        };
        if self.arbiter.as_deref() == Some("cvvdp") {
            c.arbiter = jixel::Arbiter::Cvvdp;
        }
        for (on, off, value) in [
            (self.patches, self.no_patches, &mut c.patches),
            (self.splines, self.no_splines, &mut c.splines),
            (self.edge_splines, self.no_edge_splines, &mut c.edge_splines),
            (self.dots, self.no_dots, &mut c.dots),
            (self.ink_layer, self.no_ink_layer, &mut c.ink_layer),
            (
                self.vector_layers,
                self.no_vector_layers,
                &mut c.vector_layers,
            ),
            (
                self.fill_invisible,
                self.no_fill_invisible,
                &mut c.fill_invisible,
            ),
            (self.learned_rate, self.no_learned_rate, &mut c.learned_rate),
        ] {
            if on || off {
                *value = on;
            }
        }
        c.grain = if let Some(iso) = self.photon_noise_iso {
            jixel::Grain::PhotonIso(iso)
        } else if let Some(scale) = self.measured_grain {
            jixel::Grain::Measured(scale)
        } else if let Some(lut) = &self.grain_lut {
            jixel::Grain::Lut(
                lut.as_slice()
                    .try_into()
                    .map_err(|_| invalid("--grain-lut requires exactly eight strengths"))?,
            )
        } else if self.grain.as_deref() == Some("auto") {
            jixel::Grain::Auto
        } else {
            jixel::Grain::Off
        };
        c.intensity_target = self.intensity_target;
        if let Some(shifts) = &c.progressive_shifts
            && (shifts.is_empty()
                || shifts.len() > 4
                || shifts.last() != Some(&0)
                || shifts.iter().any(|&s| s > 3)
                || shifts.windows(2).any(|w| w[0] <= w[1]))
        {
            return Err(invalid(
                "--progressive-shifts needs 1..4 strictly descending shifts in 0..3 ending in 0",
            ));
        }
        Ok(c)
    }

    pub fn validate_mode(
        &self,
        matches: &clap::ArgMatches,
        transcode: bool,
    ) -> Result<(), clap::Error> {
        if !transcode && self.no_jpeg_reconstruction {
            return Err(invalid(
                "--no-jpeg-reconstruction requires JPEG transcoding",
            ));
        }
        let lossy_only = [
            "alpha_distance",
            "fill_invisible",
            "no_fill_invisible",
            "progressive_passes",
            "progressive_shifts",
            "progressive_dc",
            "progressive_dc_levels",
            "group_order",
            "coding",
            "lossy_modular",
            "arbiter",
            "learned_rate",
            "no_learned_rate",
            "splines",
            "no_splines",
            "edge_splines",
            "no_edge_splines",
            "dots",
            "no_dots",
            "ink_layer",
            "no_ink_layer",
            "vector_layers",
            "no_vector_layers",
            "grain",
            "photon_noise_iso",
            "measured_grain",
            "grain_lut",
        ];
        for argument in Self::command().get_arguments() {
            let name = argument.get_id().as_str();
            if matches.value_source(name) != Some(clap::parser::ValueSource::CommandLine) {
                continue;
            }
            if transcode
                && ![
                    "input",
                    "output",
                    "jpeg",
                    "no_jpeg_reconstruction",
                    "speed",
                    "threads",
                    "force",
                    "quiet",
                    "recursive",
                    "jobs",
                    "min_long_edge",
                    "name",
                ]
                .contains(&name)
            {
                return Err(invalid(format!(
                    "--{} is not available in JPEG transcoding; choose -q/-d or --lossless for pixel encoding",
                    name.replace('_', "-")
                )));
            }
            if self.lossless && lossy_only.contains(&name) {
                return Err(invalid(format!(
                    "--{} is only available in lossy pixel encoding",
                    name.replace('_', "-")
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(options: &[&str]) -> Args {
        Args::try_parse_from(
            ["jixel", "input.png", "output.jxl"]
                .into_iter()
                .chain(options.iter().copied()),
        )
        .unwrap()
    }

    #[test]
    fn command_is_consistent_and_help_is_successful() {
        Args::command().debug_assert();
        for flag in ["-h", "--help", "--version"] {
            let error = Args::try_parse_from(["jixel", flag]).unwrap_err();
            assert!(!error.use_stderr());
        }
        let mut command = Args::command();
        let short = command.render_help().to_string();
        let long = command.render_long_help().to_string();
        assert!(short.contains("EXAMPLES:"));
        assert!(!short.contains("--grain-lut"));
        assert!(long.contains("--grain-lut"));
        assert!(long.contains("Quality 100 is still lossy"));
        for help in [
            Args::command().term_width(80).render_help().to_string(),
            Args::command()
                .term_width(80)
                .render_long_help()
                .to_string(),
        ] {
            assert!(help.lines().all(|line| line.chars().count() <= 80));
        }
    }

    #[test]
    fn modes_and_all_speeds_map_without_changing_presets() {
        let args = parse(&[]);
        assert!(!args.transcode(false));
        assert!(args.transcode(true));
        assert_eq!(args.config().unwrap().distance, 1.);
        for option in ["-q", "-d"] {
            assert!(!parse(&[option, "1"]).transcode(true));
        }
        assert!(!parse(&["--lossless"]).transcode(true));
        let names = [
            "ultrafast",
            "fastest",
            "fast",
            "medium",
            "medium-slow",
            "slow",
            "extra-slow",
        ];
        let expected = [
            Speed::UltraFast,
            Speed::Fastest,
            Speed::Fast,
            Speed::Medium,
            Speed::MediumSlow,
            Speed::Slow,
            Speed::ExtraSlow,
        ];
        for (name, speed) in names.into_iter().zip(expected) {
            let actual = parse(&["-s", name]).config().unwrap();
            let preset = EncodeConfig::default().with_speed(speed);
            assert_eq!(actual.speed, speed);
            assert_eq!(
                (actual.patches, actual.ink_layer, actual.vector_layers),
                (preset.patches, preset.ink_layer, preset.vector_layers)
            );
        }
        let c = parse(&[
            "--no-patches",
            "--no-ink-layer",
            "--no-vector-layers",
            "-s",
            "extra-slow",
        ])
        .config()
        .unwrap();
        assert!(!c.patches && !c.ink_layer && !c.vector_layers);
        assert!(!parse(&["-q", "100"]).config().unwrap().lossless);
    }

    #[test]
    fn reject_invalid_numbers_conflicts_and_unknown_options() {
        for options in [
            vec!["-q", "NaN"],
            vec!["-d", "inf"],
            vec!["-q", "101"],
            vec!["-d", "0"],
            vec!["-d", "26"],
            vec!["--alpha-distance", "NaN"],
            vec!["--intensity-target", "NaN"],
            vec!["--threads", "-1"],
            vec!["--speed", "7"],
            vec!["--unknown"],
            vec!["-q", "90", "--lossless"],
            vec!["--jpeg", "-d", "1"],
            vec!["--patches", "--no-patches"],
            vec!["--near-lossless", "2"],
            vec!["--progressive-passes", "5"],
            vec!["--grain", "auto", "--measured-grain", "1"],
            vec!["--strip-metadata", "--exif", "file"],
            vec!["--icc", "file", "--color-encoding", "srgb"],
        ] {
            assert!(
                Args::try_parse_from(
                    ["jixel", "input", "output"]
                        .into_iter()
                        .chain(options.iter().copied())
                )
                .is_err(),
                "{options:?}"
            );
        }
        for options in [
            vec!["--progressive-shifts", "2,0,1"],
            vec!["--progressive-shifts", "0,0"],
            vec!["--grain-lut", "0,1"],
            vec!["--progressive-shifts", "4,0"],
        ] {
            assert!(parse(&options).config().is_err(), "{options:?}");
        }
    }

    #[test]
    fn advanced_config_reaches_the_library() {
        let c = parse(&[
            "-s",
            "slow",
            "--alpha-distance",
            "0",
            "--progressive-shifts",
            "2,1,0",
            "--progressive-dc-levels",
            "3",
            "--group-order",
            "diagonals",
            "--arbiter",
            "cvvdp",
            "--coding",
            "rgb",
            "--no-learned-rate",
            "--no-fill-invisible",
            "--grain-lut",
            "0,0.1,0.2,0.3,0.4,0.5,0.6,1",
        ])
        .config()
        .unwrap();
        assert_eq!(c.alpha_distance, Some(0.));
        assert_eq!(c.progressive_shifts, Some(vec![2, 1, 0]));
        assert!(c.progressive_dc);
        assert_eq!(c.progressive_dc_levels, Some(3));
        assert_eq!(c.group_order, jixel::GroupOrder::Diagonals);
        assert_eq!(c.arbiter, jixel::Arbiter::Cvvdp);
        assert_eq!(c.coding_transform, jixel::CodingTransform::Rgb);
        assert!(!c.learned_rate && !c.fill_invisible);
        assert!(matches!(c.grain, jixel::Grain::Lut(_)));
    }
}
