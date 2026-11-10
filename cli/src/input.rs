use crate::{
    Result,
    args::{Args, invalid},
};
use image::ImageDecoder;
use jixel::{
    ColorEncoding, EncodeConfig, Orientation, Primaries, RenderingIntent, TransferFunction,
    WhitePoint,
};
use moxcms::{ColorProfile, DataColorSpace, Layout, TransformOptions};
use std::io::Cursor;

fn read_metadata(path: &std::path::Path, kind: &str) -> Result<Vec<u8>> {
    Ok(std::fs::read(path)
        .map_err(|error| format!("cannot read {kind} '{}': {error}", path.display()))?)
}

pub enum Pixels {
    U8(Vec<u8>),
    U16(Vec<u16>),
}

pub struct Input {
    pub width: usize,
    pub height: usize,
    pub channels: usize,
    pub pixels: Pixels,
    icc: Option<Vec<u8>>,
    encoding: ColorEncoding,
    exif: Option<Vec<u8>>,
    xmp: Option<Vec<u8>>,
}

pub fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xff, 0xd8, 0xff])
}

pub fn dimensions(bytes: &[u8]) -> Result<(u32, u32)> {
    if is_jpeg(bytes) {
        let decoder = image::codecs::jpeg::JpegDecoder::new(Cursor::new(bytes))?;
        Ok(decoder.dimensions())
    } else {
        let reader = png::Decoder::new(Cursor::new(bytes)).read_info()?;
        Ok((reader.info().width, reader.info().height))
    }
}

fn decoded_buffer(len: usize) -> Result<Vec<u8>> {
    if image::Limits::default()
        .max_alloc
        .is_some_and(|limit| len as u64 > limit)
    {
        return Err("decoded image exceeds the 512 MiB decoder allocation limit".into());
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(len)?;
    bytes.resize(len, 0);
    Ok(bytes)
}

impl Input {
    pub fn decode(bytes: &[u8], args: &Args) -> Result<Self> {
        let mut input = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            Self::png(bytes, args)?
        } else if is_jpeg(bytes) {
            Self::jpeg(bytes, args)?
        } else {
            return Err("expected a PNG or JPEG file (detected by content)".into());
        };
        if let Some(path) = &args.icc {
            input.icc = Some(read_metadata(path, "ICC profile")?);
        }
        if let Some(name) = &args.color_encoding {
            input.encoding = named_encoding(name);
            input.icc = None;
        }
        if let Some(path) = &args.exif {
            let exif = read_metadata(path, "EXIF")?;
            if exif.len() < 8 || !(exif.starts_with(b"II\x2a\0") || exif.starts_with(b"MM\0\x2a")) {
                return Err(
                    invalid("--exif requires raw TIFF bytes, without an Exif prefix").into(),
                );
            }
            input.exif = Some(exif);
        }
        if let Some(path) = &args.xmp {
            let xmp = read_metadata(path, "XMP")?;
            std::str::from_utf8(&xmp).map_err(|_| invalid("--xmp requires a UTF-8 XMP packet"))?;
            input.xmp = Some(xmp);
        }
        Ok(input)
    }

    fn png(bytes: &[u8], args: &Args) -> Result<Self> {
        let mut decoder = png::Decoder::new(Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = decoder.read_info()?;
        if reader.info().animation_control.is_some() {
            return Err("APNG is not supported; supply a still PNG".into());
        }
        let mut raw = decoded_buffer(
            reader
                .output_buffer_size()
                .ok_or("PNG dimensions overflow addressable memory")?,
        )?;
        let output = reader.next_frame(&mut raw)?;
        raw.truncate(output.buffer_size());
        reader.finish()?;
        let info = reader.info();
        let channels = output.color_type.samples();
        let mut icc = info.icc_profile.as_ref().map(|p| p.to_vec());
        let mut encoding = ColorEncoding::srgb();
        if args.color_encoding.is_none() && args.icc.is_none() {
            if let Some(cicp) = info.coding_independent_code_points {
                if cicp.matrix_coefficients != 0 || !cicp.is_video_full_range_image {
                    return Err(
                        "PNG cICP requires RGB matrix coefficients and full-range samples".into(),
                    );
                }
                encoding = cicp_encoding(cicp.color_primaries, cicp.transfer_function)?;
                // cICP has precedence over iCCP in PNG.
                icc = None;
            } else if icc.is_none()
                && info.srgb.is_none()
                && (info.gama_chunk.is_some() || info.chrm_chunk.is_some())
            {
                icc = Some(png_profile(info, channels <= 2)?.encode()?);
            }
        }
        let mut xmp = None;
        for text in &info.utf8_text {
            if text.keyword == "XML:com.adobe.xmp" {
                xmp = Some(text.get_text()?.into_bytes());
                break;
            }
        }
        if xmp.is_none() {
            for text in &info.uncompressed_latin1_text {
                if text.keyword == "XML:com.adobe.xmp" {
                    xmp = Some(text.text.as_bytes().to_vec());
                    break;
                }
            }
        }
        if xmp.is_none() {
            for text in &info.compressed_latin1_text {
                if text.keyword == "XML:com.adobe.xmp" {
                    xmp = Some(text.get_text()?.into_bytes());
                    break;
                }
            }
        }
        let pixels = if output.bit_depth == png::BitDepth::Sixteen {
            Pixels::U16(
                raw.as_chunks::<2>()
                    .0
                    .iter()
                    .map(|p| u16::from_be_bytes(*p))
                    .collect(),
            )
        } else {
            Pixels::U8(raw)
        };
        Ok(Self {
            width: output.width as usize,
            height: output.height as usize,
            channels,
            pixels,
            icc,
            encoding,
            exif: info.exif_metadata.as_ref().map(|p| p.to_vec()),
            xmp,
        })
    }

    fn jpeg(bytes: &[u8], args: &Args) -> Result<Self> {
        let mut decoder = image::codecs::jpeg::JpegDecoder::new(Cursor::new(bytes))?;
        decoder.set_limits(image::Limits::default())?;
        let (width, height) = decoder.dimensions();
        let channels = usize::from(decoder.color_type().channel_count());
        let icc = decoder.icc_profile()?;
        let exif = decoder.exif_metadata()?;
        let xmp = if args.strip_metadata || args.xmp.is_some() {
            None
        } else {
            // ImageDecoder exposes only standard XMP; its JPEG backend also knows extensions.
            let options = zune_jpeg::zune_core::options::DecoderOptions::default()
                .set_strict_mode(false)
                .set_max_width(usize::MAX)
                .set_max_height(usize::MAX);
            let mut metadata = zune_jpeg::JpegDecoder::new_with_options(
                zune_jpeg::zune_core::bytestream::ZCursor::new(bytes),
                options,
            );
            metadata.decode_headers()?;
            let xmp = metadata.xmp().cloned();
            if metadata
                .info()
                .is_some_and(|info| info.extended_xmp.is_some())
            {
                return Err("JPEG Extended XMP cannot be preserved in pixel mode; use --strip-metadata or --xmp FILE to replace it".into());
            }
            xmp
        };
        let mut pixels = decoded_buffer(usize::try_from(decoder.total_bytes())?)?;
        decoder.read_image(&mut pixels)?;
        Ok(Self {
            width: width as usize,
            height: height as usize,
            channels,
            pixels: Pixels::U8(pixels),
            icc,
            exif,
            xmp,
            encoding: ColorEncoding::srgb(),
        })
    }

    pub fn configure(&mut self, args: &Args, c: &mut EncodeConfig) -> Result<()> {
        c.color_encoding = self.encoding;
        if let Some(icc) = &self.icc {
            let profile = ColorProfile::new_from_slice(icc)?;
            let expected = if self.channels <= 2 {
                DataColorSpace::Gray
            } else {
                DataColorSpace::Rgb
            };
            if profile.color_space != expected {
                return Err("ICC color space does not match decoded channels (CMYK pixel input is not supported)".into());
            }
            let hdr = profile
                .cicp
                .filter(|p| matches!(p.transfer_characteristics as u8, 16 | 18));
            if let Some(hdr) = hdr {
                c.color_encoding = cicp_encoding(
                    hdr.color_primaries as u8,
                    hdr.transfer_characteristics as u8,
                )?;
            }
            if c.lossless || hdr.is_some() {
                c.icc_profile = Some(icc.clone());
            } else {
                self.convert_to_srgb(&profile)?;
                c.color_encoding = ColorEncoding::srgb();
            }
        }
        c.orientation = self
            .exif
            .as_deref()
            .and_then(image::metadata::Orientation::from_exif_chunk)
            .and_then(|o| Orientation::from_exif(o.to_exif()))
            .unwrap_or_default();
        if !args.strip_metadata {
            c.exif = self.exif.clone();
            c.xmp = self.xmp.clone();
        }
        let peak = match c.color_encoding.transfer {
            TransferFunction::Smpte2084 => Some(10000.),
            TransferFunction::Hlg => Some(1000.),
            _ => None,
        };
        if c.intensity_target.is_none() {
            c.intensity_target = peak;
        }
        let max_nits = c.intensity_target.unwrap_or(255.);
        if !(0.000000059604645..=65504.).contains(&max_nits) {
            return Err(invalid("--intensity-target must fit a positive finite half float (approximately 0.00000006..65504 nits)").into());
        }
        self.validate(args, c)?;
        Ok(())
    }

    fn convert_to_srgb(&mut self, profile: &ColorProfile) -> Result<()> {
        let layout = match self.channels {
            1 => Layout::Gray,
            2 => Layout::GrayAlpha,
            3 => Layout::Rgb,
            _ => Layout::Rgba,
        };
        let mut target = ColorProfile::new_srgb();
        if self.channels <= 2 {
            let trc = target.red_trc.clone();
            target = ColorProfile::new_gray_with_gamma(1.);
            target.gray_trc = trc;
        }
        let options = TransformOptions {
            rendering_intent: moxcms::RenderingIntent::RelativeColorimetric,
            ..Default::default()
        };
        match &mut self.pixels {
            Pixels::U8(data) => {
                let transform = profile.create_transform_8bit(layout, &target, layout, options)?;
                let mut output = vec![0; data.len()];
                transform.transform(data, &mut output)?;
                preserve_alpha(data, &mut output, self.channels);
                *data = output;
            }
            Pixels::U16(data) => {
                let transform = profile.create_transform_16bit(layout, &target, layout, options)?;
                let mut output = vec![0; data.len()];
                transform.transform(data, &mut output)?;
                preserve_alpha(data, &mut output, self.channels);
                *data = output;
            }
        }
        Ok(())
    }

    fn validate(&self, a: &Args, c: &EncodeConfig) -> Result<()> {
        let alpha = self.channels == 2 || self.channels == 4;
        let gray = self.channels <= 2;
        let bit8 = matches!(self.pixels, Pixels::U8(_));
        if bit8 && c.near_lossless > 255 {
            return Err(invalid("--near-lossless must be <=255 for 8-bit input").into());
        }
        if a.alpha_distance.is_some() && !alpha {
            return Err(invalid("--alpha-distance requires an input alpha channel").into());
        }
        if a.decoding_speed.is_some() && !c.lossless && !alpha {
            return Err(
                invalid("--decoding-speed requires lossless encoding or an alpha channel").into(),
            );
        }
        let progressive = c.progressive
            || c.progressive_passes.is_some_and(|p| p > 1)
            || c.progressive_shifts.as_ref().is_some_and(|s| s.len() > 1);
        if c.progressive_dc
            && (c.lossless || alpha || gray || c.coding_transform != jixel::CodingTransform::Xyb)
        {
            return Err(invalid("--progressive-dc requires lossy XYB RGB without alpha").into());
        }
        if c.lossy_modular != jixel::LossyModular::Off
            && (progressive || c.progressive_dc || a.group_order.is_some())
        {
            return Err(invalid(
                "lossy modular cannot be combined with progressive or group-order options",
            )
            .into());
        }
        if c.lossy_modular == jixel::LossyModular::Auto && c.speed < jixel::Speed::Slow {
            return Err(invalid("--lossy-modular auto requires slow or extra-slow").into());
        }
        if a.learned_rate && c.speed < jixel::Speed::MediumSlow {
            return Err(invalid("--learned-rate requires medium-slow or slower").into());
        }
        if (a.splines || a.edge_splines || a.dots)
            && (c.speed < jixel::Speed::Slow
                || c.coding_transform != jixel::CodingTransform::Xyb
                || c.decoding_speed != jixel::DecodingSpeed::Slow
                || c.lossy_modular != jixel::LossyModular::Off)
        {
            return Err(invalid("explicit spline/dot tools require slow/extra-slow lossy XYB VarDCT and decoding-speed slow").into());
        }
        if a.edge_splines && (c.distance <= 1. || progressive || c.progressive_dc) {
            return Err(
                invalid("--edge-splines requires distance >1 and a non-progressive frame").into(),
            );
        }
        if a.ink_layer
            && (!bit8
                || gray
                || c.coding_transform != jixel::CodingTransform::Xyb
                || c.progressive_dc)
        {
            return Err(
                invalid("--ink-layer requires 8-bit RGB/RGBA XYB without progressive DC").into(),
            );
        }
        if a.vector_layers && (!bit8 || self.channels != 3) {
            return Err(invalid("--vector-layers requires 8-bit RGB input").into());
        }
        if c.grain != jixel::Grain::Off && c.lossy_modular != jixel::LossyModular::Off {
            return Err(invalid("grain synthesis requires VarDCT (--lossy-modular off)").into());
        }
        Ok(())
    }

    pub fn encode(&self, c: &EncodeConfig) -> Result<Vec<u8>> {
        let (w, h) = (self.width, self.height);
        let result = match (&self.pixels, self.channels) {
            (Pixels::U8(p), 1) => jixel::encode_image_gray(p, w, h, c),
            (Pixels::U8(p), 2) => jixel::encode_image_gray_alpha(p, w, h, c),
            (Pixels::U8(p), 3) => jixel::encode_image(p, w, h, c),
            (Pixels::U8(p), 4) => jixel::encode_image_with_alpha(p, w, h, c),
            (Pixels::U16(p), 1) => jixel::encode_image_gray_16bit(p, w, h, c),
            (Pixels::U16(p), 2) => jixel::encode_image_gray_alpha_16bit(p, w, h, c),
            (Pixels::U16(p), 3) => jixel::encode_image_16bit(p, w, h, c),
            (Pixels::U16(p), 4) => jixel::encode_image_with_alpha_16bit(p, w, h, c),
            _ => return Err("unsupported channel layout".into()),
        };
        Ok(result?)
    }
}

fn preserve_alpha<T: Copy>(input: &[T], output: &mut [T], channels: usize) {
    if channels == 2 || channels == 4 {
        for (src, dst) in input
            .chunks_exact(channels)
            .zip(output.chunks_exact_mut(channels))
        {
            dst[channels - 1] = src[channels - 1];
        }
    }
}

fn cicp_encoding(primaries: u8, transfer: u8) -> Result<ColorEncoding> {
    let primaries = match primaries {
        1 => Primaries::Bt709,
        9 => Primaries::Bt2020,
        12 => Primaries::Smpte431,
        _ => {
            return Err(
                format!("unsupported cICP primaries {primaries}; expected 1, 9 or 12").into(),
            );
        }
    };
    let transfer = match transfer {
        1 | 6 | 14 | 15 => TransferFunction::Bt709,
        8 => TransferFunction::Linear,
        13 => TransferFunction::Srgb,
        16 => TransferFunction::Smpte2084,
        18 => TransferFunction::Hlg,
        _ => return Err(format!("unsupported cICP transfer {transfer}").into()),
    };
    Ok(ColorEncoding {
        white_point: WhitePoint::D65,
        primaries,
        transfer,
        rendering_intent: RenderingIntent::Relative,
    })
}

fn named_encoding(name: &str) -> ColorEncoding {
    match name {
        "linear-srgb" => ColorEncoding::srgb_linear(),
        "display-p3" => ColorEncoding::display_p3(),
        "bt2020" => ColorEncoding {
            primaries: Primaries::Bt2020,
            transfer: TransferFunction::Bt202010bit,
            ..ColorEncoding::srgb()
        },
        "bt2100-pq" => ColorEncoding::bt2020_pq(),
        "bt2100-hlg" => ColorEncoding::bt2020_hlg(),
        _ => ColorEncoding::srgb(),
    }
}

fn png_profile(info: &png::Info<'_>, gray: bool) -> Result<ColorProfile> {
    let gamma = info.gama_chunk.map(|g| g.into_value());
    if gamma.is_some_and(|g| !g.is_finite() || g <= 0. || !(1. / g).is_finite()) {
        return Err("invalid PNG gamma".into());
    }
    let mut profile = if gray {
        ColorProfile::new_gray_with_gamma(1.)
    } else {
        ColorProfile::new_srgb()
    };
    // A gamma override must not retain the constructor's sRGB cICP transfer.
    profile.cicp = None;
    let curve = match gamma {
        Some(g) => moxcms::ToneReprCurve::Parametric(vec![1. / g]),
        None => ColorProfile::new_srgb().red_trc.unwrap(),
    };
    if gray {
        profile.gray_trc = Some(curve);
    } else {
        profile.red_trc = Some(curve.clone());
        profile.green_trc = Some(curve.clone());
        profile.blue_trc = Some(curve);
    }
    if let Some(chrm) = info.chrm_chunk {
        let xy = |p: (png::ScaledFloat, png::ScaledFloat)| -> Result<moxcms::Chromaticity> {
            let (x, y) = (p.0.into_value(), p.1.into_value());
            if x <= 0. || y <= 0. || x + y > 1.00001 {
                return Err("invalid PNG chromaticity".into());
            }
            Ok(moxcms::Chromaticity { x, y })
        };
        let white = xy(chrm.white)?;
        if gray {
            // Gray profiles here use D65; do not silently relabel another white point.
            if (white.x - 0.3127).abs() > 0.0001 || (white.y - 0.329).abs() > 0.0001 {
                return Err("non-D65 grayscale PNG requires an ICC profile".into());
            }
        } else {
            profile.update_rgb_colorimetry(
                moxcms::XyY {
                    x: f64::from(white.x),
                    y: f64::from(white.y),
                    yb: 1.,
                },
                moxcms::ColorPrimaries {
                    red: xy(chrm.red)?,
                    green: xy(chrm.green)?,
                    blue: xy(chrm.blue)?,
                },
            );
            let m = profile.colorant_matrix();
            if !m
                .determinant()
                .is_some_and(|d| d.is_finite() && d.abs() > 1e-10)
            {
                return Err("degenerate PNG chromaticities".into());
            }
        }
    }
    Ok(profile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn args(options: &[&str]) -> Args {
        Args::try_parse_from(
            ["jixel", "in", "out"]
                .into_iter()
                .chain(options.iter().copied()),
        )
        .unwrap()
    }

    fn input(pixels: Pixels, channels: usize, icc: Option<Vec<u8>>) -> Input {
        Input {
            width: 1,
            height: 1,
            channels,
            pixels,
            icc,
            encoding: ColorEncoding::srgb(),
            exif: None,
            xmp: None,
        }
    }

    #[test]
    fn color_conversion_changes_sdr_samples_but_keeps_depth_and_alpha() {
        let a = args(&[]);
        let icc = ColorProfile::new_display_p3().encode().unwrap();
        let mut image = input(Pixels::U8(vec![200, 100, 100, 73]), 4, Some(icc.clone()));
        let mut c = a.config().unwrap();
        image.configure(&a, &mut c).unwrap();
        let Pixels::U8(actual) = &image.pixels else {
            panic!("lost 8-bit depth")
        };
        // Display P3 -> sRGB matrix and sRGB transfer, rounded to code values.
        for (&a, b) in actual.iter().zip([215u8, 93, 97, 73]) {
            assert!(a.abs_diff(b) <= 2, "{actual:?}");
        }
        assert_eq!(actual[3], 73);
        assert!(c.icc_profile.is_none());
        assert_eq!(c.color_encoding.primaries, Primaries::Bt709);
        let mut image = input(Pixels::U16(vec![50000, 20000, 20000, 12345]), 4, Some(icc));
        image.configure(&a, &mut c).unwrap();
        let Pixels::U16(actual) = &image.pixels else {
            panic!("lost 16-bit depth")
        };
        assert!(actual[0] > 50000 && actual[1] < 20000);
        assert_eq!(actual[3], 12345);
        let mut gray = input(
            Pixels::U8(vec![128, 73]),
            2,
            Some(ColorProfile::new_gray_with_gamma(1.).encode().unwrap()),
        );
        gray.configure(&a, &mut c).unwrap();
        let Pixels::U8(actual) = &gray.pixels else {
            panic!()
        };
        assert!(actual[0].abs_diff(188) <= 1);
        assert_eq!(actual[1], 73);
    }

    #[test]
    fn lossless_retains_icc_and_samples_and_strip_keeps_orientation() {
        let a = args(&["--lossless", "--strip-metadata"]);
        let icc = ColorProfile::new_display_p3().encode().unwrap();
        let mut image = input(Pixels::U16(vec![50000, 20000, 20000]), 3, Some(icc.clone()));
        image.exif =
            Some(b"II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0".to_vec());
        let mut c = a.config().unwrap();
        image.configure(&a, &mut c).unwrap();
        assert_eq!(c.icc_profile, Some(icc));
        assert!(matches!(image.pixels,Pixels::U16(ref p) if p==&[50000,20000,20000]));
        assert_eq!(c.orientation, Orientation::Rotate90);
        assert!(c.exif.is_none());
    }

    #[test]
    fn png_gamma_is_interpreted_and_bad_color_descriptions_fail() {
        let mut info = png::Info::with_size(1, 1);
        info.gama_chunk = Some(png::ScaledFloat::new(1.));
        let profile = png_profile(&info, false).unwrap();
        let a = args(&[]);
        let mut image = input(
            Pixels::U8(vec![128, 128, 128]),
            3,
            Some(profile.encode().unwrap()),
        );
        image.configure(&a, &mut a.config().unwrap()).unwrap();
        assert!(matches!(image.pixels,Pixels::U8(ref p) if p.iter().all(|v|v.abs_diff(188)<=1)));
        info.gama_chunk = Some(png::ScaledFloat::new(0.));
        assert!(png_profile(&info, false).is_err());
        assert!(cicp_encoding(99, 16).is_err());
        assert!(cicp_encoding(9, 99).is_err());
        let mut image = input(
            Pixels::U8(vec![128]),
            1,
            Some(ColorProfile::new_srgb().encode().unwrap()),
        );
        assert!(image.configure(&a, &mut a.config().unwrap()).is_err());
    }

    #[test]
    fn named_spaces_and_hdr_cicp_preserve_transfer_and_luminance() {
        let a = args(&[]);
        for (name, transfer, peak) in [
            ("bt2100-pq", TransferFunction::Smpte2084, 10000.),
            ("bt2100-hlg", TransferFunction::Hlg, 1000.),
        ] {
            let mut image = input(Pixels::U16(vec![10000, 20000, 30000]), 3, None);
            image.encoding = named_encoding(name);
            let mut c = a.config().unwrap();
            image.configure(&a, &mut c).unwrap();
            assert_eq!(c.color_encoding.transfer, transfer);
            assert_eq!(c.intensity_target, Some(peak));
        }
    }
}
