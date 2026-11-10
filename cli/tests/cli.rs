use std::{
    borrow::Cow,
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_jixel"))
}

fn succeeds(output: Output) -> Output {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn samples(channels: usize, bits: u8) -> Vec<u8> {
    let max = if bits == 16 { 65535 } else { 255 };
    (0..16 * 16 * channels)
        .flat_map(|i| {
            let v = ((i * 137 + 17) % max) as u16;
            if bits == 16 {
                v.to_be_bytes().to_vec()
            } else {
                vec![v as u8]
            }
        })
        .collect()
}

fn png(
    path: &Path,
    channels: usize,
    bits: u8,
    icc: Option<Vec<u8>>,
    exif: Option<Vec<u8>>,
) -> Vec<u8> {
    let raw = samples(channels, bits);
    let mut info = png::Info::with_size(16, 16);
    info.color_type = match channels {
        1 => png::ColorType::Grayscale,
        2 => png::ColorType::GrayscaleAlpha,
        3 => png::ColorType::Rgb,
        _ => png::ColorType::Rgba,
    };
    info.bit_depth = if bits == 16 {
        png::BitDepth::Sixteen
    } else {
        png::BitDepth::Eight
    };
    info.icc_profile = icc.map(Cow::Owned);
    info.exif_metadata = exif.map(Cow::Owned);
    let mut encoder = png::Encoder::with_info(fs::File::create(path).unwrap(), info).unwrap();
    encoder
        .add_itxt_chunk(
            "XML:com.adobe.xmp".into(),
            "<x:xmpmeta xmlns:x='adobe:ns:meta/'/>".into(),
        )
        .unwrap();
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&raw)
        .unwrap();
    raw
}

fn jpeg(path: &Path) {
    image::codecs::jpeg::JpegEncoder::new_with_quality(fs::File::create(path).unwrap(), 90)
        .encode(&samples(3, 8), 16, 16, image::ExtendedColorType::Rgb8)
        .unwrap();
}

fn extended_xmp_jpeg(path: &Path) {
    jpeg(path);
    let original = fs::read(path).unwrap();
    let guid = b"21901916FC484D8D53A5571E70495C9E";
    let extended = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:dc="http://purl.org/dc/elements/1.1/" dc:description="extended content"/></rdf:RDF></x:xmpmeta>"#;
    let standard = format!(
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:xmpNote="http://ns.adobe.com/xmp/note/" xmpNote:HasExtendedXMP="{}"/></rdf:RDF></x:xmpmeta>"#,
        std::str::from_utf8(guid).unwrap()
    );
    let mut bytes = original[..2].to_vec();
    let mut app1 = |payload: &[u8]| {
        bytes.extend_from_slice(&[0xff, 0xe1]);
        bytes.extend_from_slice(&u16::try_from(payload.len() + 2).unwrap().to_be_bytes());
        bytes.extend_from_slice(payload);
    };
    app1(
        &[
            b"http://ns.adobe.com/xap/1.0/\0".as_slice(),
            standard.as_bytes(),
        ]
        .concat(),
    );
    // Reverse two chunks to exercise the decoder's extension reassembly.
    let split = extended.len() / 2;
    for (offset, data) in [(split, &extended[split..]), (0, &extended[..split])] {
        let mut payload = b"http://ns.adobe.com/xmp/extension/\0".to_vec();
        payload.extend_from_slice(guid);
        payload.extend_from_slice(&(extended.len() as u32).to_be_bytes());
        payload.extend_from_slice(&(offset as u32).to_be_bytes());
        payload.extend_from_slice(data);
        app1(&payload);
    }
    bytes.extend_from_slice(&original[2..]);
    fs::write(path, bytes).unwrap();
}

#[test]
fn redirected_streams_cannot_modify_the_input_file() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.png");
    png(&input, 3, 8, None, None);
    let original = fs::read(&input).unwrap();
    for (stdin, stdout) in [(true, false), (false, true), (true, true)] {
        let mut cmd = cli();
        cmd.arg(if stdin { Path::new("-") } else { &input })
            .arg(if stdout { Path::new("-") } else { &input })
            .arg("--lossless");
        if stdin {
            cmd.stdin(fs::File::open(&input).unwrap());
        }
        if stdout {
            cmd.stdout(fs::OpenOptions::new().append(true).open(&input).unwrap());
        } else {
            cmd.arg("--force");
        }
        let out = cmd.output().unwrap();
        assert!(
            fs::read(&input).unwrap() == original,
            "original changed: stdin={stdin}, stdout={stdout}"
        );
        assert_eq!(out.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&out.stderr).contains("same file"));
    }
    let output = dir.path().join("output.jxl");
    for (stdin, stdout) in [(true, false), (false, true), (true, true)] {
        let mut cmd = cli();
        cmd.arg(if stdin { Path::new("-") } else { &input })
            .arg(if stdout { Path::new("-") } else { &output })
            .args(["--lossless", "--force"]);
        if stdin {
            cmd.stdin(fs::File::open(&input).unwrap());
        }
        if stdout {
            cmd.stdout(fs::File::create(&output).unwrap());
        }
        succeeds(cmd.output().unwrap());
        assert_eq!(fs::read(&input).unwrap(), original);
        assert!(!fs::read(&output).unwrap().is_empty());
    }
}

#[test]
fn ordinary_jpeg_xmp_is_preserved_even_when_it_mentions_extended_xmp() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.jpg");
    let output = dir.path().join("output.jxl");
    jpeg(&input);
    let original = fs::read(&input).unwrap();
    let packet = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><!-- HasExtendedXMP is only a comment --></x:xmpmeta>"#;
    let payload = [b"http://ns.adobe.com/xap/1.0/\0".as_slice(), packet].concat();
    let mut bytes = original[..2].to_vec();
    bytes.extend_from_slice(&[0xff, 0xe1]);
    bytes.extend_from_slice(&u16::try_from(payload.len() + 2).unwrap().to_be_bytes());
    bytes.extend_from_slice(&payload);
    bytes.extend_from_slice(&original[2..]);
    fs::write(&input, bytes).unwrap();
    succeeds(
        cli()
            .arg(&input)
            .arg(&output)
            .arg("--lossless")
            .output()
            .unwrap(),
    );
    assert!(
        fs::read(&output)
            .unwrap()
            .windows(packet.len())
            .any(|w| w == packet)
    );
}

#[test]
fn extended_xmp_requires_explicit_stripping_or_replacement_in_pixel_modes() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.jpg");
    let output = dir.path().join("output.jxl");
    let replacement = dir.path().join("replacement.xmp");
    extended_xmp_jpeg(&input);
    fs::write(&replacement, b"<replacement/>").unwrap();
    for mode in [vec!["--lossless"], vec!["-q", "90"], vec!["-d", "1"]] {
        fs::write(&output, b"keep").unwrap();
        let out = cli()
            .arg(&input)
            .arg(&output)
            .arg("--force")
            .args(&mode)
            .output()
            .unwrap();
        assert!(
            fs::read(&output).unwrap() == b"keep",
            "existing output changed despite unsupported Extended XMP"
        );
        assert_eq!(out.status.code(), Some(1));
        let message = String::from_utf8_lossy(&out.stderr);
        assert!(
            message.contains("Extended XMP")
                && message.contains("--strip-metadata")
                && message.contains("--xmp"),
            "{message}"
        );
        succeeds(
            cli()
                .arg(&input)
                .arg(&output)
                .arg("--force")
                .args(&mode)
                .arg("--strip-metadata")
                .output()
                .unwrap(),
        );
        succeeds(
            cli()
                .arg(&input)
                .arg(&output)
                .arg("--force")
                .args(&mode)
                .arg("--xmp")
                .arg(&replacement)
                .output()
                .unwrap(),
        );
        let data = fs::read(&output).unwrap();
        assert!(
            data.windows(b"<replacement/>".len())
                .any(|w| w == b"<replacement/>")
        );
        assert!(
            !data
                .windows(b"HasExtendedXMP".len())
                .any(|w| w == b"HasExtendedXMP")
        );
    }
    succeeds(
        cli()
            .arg(&input)
            .arg(&output)
            .arg("--force")
            .output()
            .unwrap(),
    );
}

fn exif(orientation: u8) -> Vec<u8> {
    let mut bytes = b"II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x01\0\0\0\0\0\0\0".to_vec();
    bytes[18] = orientation;
    bytes
}

#[test]
fn help_and_version_work_without_inputs_and_do_not_emit_ansi_in_pipes() {
    for arg in ["--help", "-h", "--version"] {
        let out = succeeds(cli().arg(arg).env("NO_COLOR", "1").output().unwrap());
        assert!(out.stderr.is_empty());
        assert!(!out.stdout.contains(&0x1b));
    }
    let out = cli().arg("--qulaity").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--help"));
}

#[test]
fn files_pipes_and_overwrite_protection() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.dat");
    let output = dir.path().join("output.jxl");
    png(&input, 4, 16, None, None);
    succeeds(
        cli()
            .arg(&input)
            .arg(&output)
            .args(["--lossless", "--quiet"])
            .output()
            .unwrap(),
    );
    let original = fs::read(&output).unwrap();
    assert_eq!(
        cli()
            .arg(&input)
            .arg(&output)
            .output()
            .unwrap()
            .status
            .code(),
        Some(1)
    );
    assert_eq!(fs::read(&output).unwrap(), original);
    succeeds(
        cli()
            .arg(&input)
            .arg(&output)
            .args(["--force", "-q", "90"])
            .output()
            .unwrap(),
    );
    assert_ne!(fs::read(&output).unwrap(), original);
    let before = fs::read(&input).unwrap();
    assert_eq!(
        cli()
            .arg(&input)
            .arg(&input)
            .arg("--force")
            .output()
            .unwrap()
            .status
            .code(),
        Some(1)
    );
    assert_eq!(fs::read(&input).unwrap(), before);
    let hardlink = dir.path().join("hardlink");
    fs::hard_link(&input, &hardlink).unwrap();
    assert_eq!(
        cli()
            .arg(&input)
            .arg(&hardlink)
            .arg("--force")
            .output()
            .unwrap()
            .status
            .code(),
        Some(1)
    );
    #[cfg(unix)]
    {
        let link = dir.path().join("symlink");
        std::os::unix::fs::symlink(&input, &link).unwrap();
        assert_eq!(
            cli()
                .arg(&input)
                .arg(&link)
                .arg("--force")
                .output()
                .unwrap()
                .status
                .code(),
            Some(1)
        );
    }
    let mut child = cli()
        .args(["-", "-", "--lossless", "--quiet"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&before).unwrap();
    let out = succeeds(child.wait_with_output().unwrap());
    assert_eq!(out.stdout, original);
    assert!(out.stderr.is_empty());
}

#[test]
fn invalid_input_and_options_never_replace_existing_output() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input");
    let output = dir.path().join("output");
    fs::write(&output, b"keep").unwrap();
    let absent = cli()
        .arg(dir.path().join("absent.png"))
        .arg(&output)
        .arg("--force")
        .output()
        .unwrap();
    assert_eq!(absent.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&absent.stderr).contains("cannot read input"));
    assert!(String::from_utf8_lossy(&absent.stderr).contains("absent.png"));
    for bytes in [
        b"bad".as_slice(),
        b"\xff\xd8\xffbroken",
        b"\x89PNG\r\n\x1a\nbroken",
    ] {
        fs::write(&input, bytes).unwrap();
        assert!(
            !cli()
                .arg(&input)
                .arg(&output)
                .arg("--force")
                .output()
                .unwrap()
                .status
                .success()
        );
        assert_eq!(fs::read(&output).unwrap(), b"keep");
    }
    png(&input, 3, 8, None, None);
    for options in [
        vec!["--alpha-distance", "0"],
        vec!["--splines"],
        vec!["--near-lossless", "300", "--lossless"],
        vec!["--lossless", "--grain", "auto"],
        vec!["--intensity-target", "70000"],
        vec!["--progressive-dc", "--coding", "rgb"],
        vec!["--no-jpeg-reconstruction"],
        vec!["--edge-splines", "-s", "slow", "-d", "1"],
    ] {
        let out = cli()
            .arg(&input)
            .arg(&output)
            .arg("--force")
            .args(&options)
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(2),
            "{options:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(fs::read(&output).unwrap(), b"keep");
    }
    let missing = dir.path().join("missing/output.jxl");
    assert_eq!(
        cli()
            .arg(&input)
            .arg(missing)
            .output()
            .unwrap()
            .status
            .code(),
        Some(1)
    );
    assert!(
        cli()
            .arg(&input)
            .arg(dir.path())
            .arg("--force")
            .output()
            .unwrap()
            .status
            .code()
            == Some(1)
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn jpeg_defaults_to_reconstruction_and_requires_explicit_pixel_mode_for_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.jpg");
    let output = dir.path().join("output.jxl");
    jpeg(&input);
    succeeds(cli().arg(&input).arg(&output).output().unwrap());
    assert!(fs::read(&output).unwrap().windows(4).any(|w| w == b"jbrd"));
    succeeds(
        cli()
            .arg(&input)
            .arg(&output)
            .args(["--force", "--no-jpeg-reconstruction"])
            .output()
            .unwrap(),
    );
    assert!(!fs::read(&output).unwrap().windows(4).any(|w| w == b"jbrd"));
    for options in [
        vec!["--strip-metadata"],
        vec!["--progressive"],
        vec!["--color-encoding", "srgb"],
    ] {
        assert_eq!(
            cli()
                .arg(&input)
                .arg(&output)
                .arg("--force")
                .args(options)
                .output()
                .unwrap()
                .status
                .code(),
            Some(2)
        );
    }
    succeeds(
        cli()
            .arg(&input)
            .arg(&output)
            .args(["--force", "-q", "90", "--strip-metadata"])
            .output()
            .unwrap(),
    );
    succeeds(
        cli()
            .arg(&input)
            .arg(&output)
            .args(["--force", "--lossless"])
            .output()
            .unwrap(),
    );
}

fn decode_png(path: &Path) -> (png::OutputInfo, Vec<u8>) {
    let mut reader = png::Decoder::new(std::io::BufReader::new(fs::File::open(path).unwrap()))
        .read_info()
        .unwrap();
    let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut bytes).unwrap();
    bytes.truncate(info.buffer_size());
    (info, bytes)
}

fn djxl(input: &Path, output: &Path) {
    succeeds(
        Command::new("djxl")
            .arg(input)
            .arg(output)
            .output()
            .expect("install libjxl tools to run ignored decoder tests"),
    );
}

#[test]
#[ignore = "requires djxl and jxlinfo from libjxl tools"]
fn independent_decoder_checks_all_lossless_layouts_and_near_lossless_bound() {
    let dir = tempfile::tempdir().unwrap();
    for bits in [8, 16] {
        for channels in 1..=4 {
            let input = dir.path().join("input.png");
            let encoded = dir.path().join("encoded.jxl");
            let decoded = dir.path().join("decoded.png");
            let raw = png(&input, channels, bits, None, None);
            for options in [
                vec!["--lossless"],
                vec!["--lossless", "--progressive"],
                vec!["--lossless", "--near-lossless", "3"],
            ] {
                succeeds(
                    cli()
                        .arg(&input)
                        .arg(&encoded)
                        .arg("--force")
                        .args(&options)
                        .output()
                        .unwrap(),
                );
                djxl(&encoded, &decoded);
                let (info, actual) = decode_png(&decoded);
                assert_eq!(info.color_type.samples(), channels);
                assert_eq!(info.bit_depth as u8, bits);
                let samples = |bytes: &[u8]| -> Vec<u16> {
                    if bits == 8 {
                        bytes.iter().map(|&x| u16::from(x)).collect()
                    } else {
                        bytes
                            .as_chunks::<2>()
                            .0
                            .iter()
                            .map(|v| u16::from_be_bytes(*v))
                            .collect()
                    }
                };
                for (i, (a, b)) in samples(&raw).into_iter().zip(samples(&actual)).enumerate() {
                    let bound = if options.contains(&"--near-lossless")
                        && !(channels % 2 == 0 && i % channels == channels - 1)
                    {
                        3
                    } else {
                        0
                    };
                    assert!(a.abs_diff(b) <= bound, "{bits}/{channels}: {a} != {b}");
                }
            }
        }
    }
}

#[test]
#[ignore = "requires djxl and jxlinfo from libjxl tools"]
fn independent_decoder_restores_original_jpeg_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.jpg");
    let encoded = dir.path().join("encoded.jxl");
    let restored = dir.path().join("restored.jpg");
    for extended in [false, true] {
        if extended {
            extended_xmp_jpeg(&input);
        } else {
            jpeg(&input);
        }
        for speed in ["fast", "slow", "extra-slow"] {
            succeeds(
                cli()
                    .arg(&input)
                    .arg(&encoded)
                    .args(["--force", "-s", speed])
                    .output()
                    .unwrap(),
            );
            djxl(&encoded, &restored);
            assert_eq!(fs::read(&input).unwrap(), fs::read(&restored).unwrap());
        }
    }
}

#[test]
fn metadata_and_icc_are_kept_or_stripped_without_relabeling_lossy_sdr() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.png");
    let output = dir.path().join("output.jxl");
    let profile = moxcms::ColorProfile::new_display_p3().encode().unwrap();
    png(&input, 4, 16, Some(profile.clone()), Some(exif(6)));
    succeeds(
        cli()
            .arg(&input)
            .arg(&output)
            .arg("--lossless")
            .output()
            .unwrap(),
    );
    let bytes = fs::read(&output).unwrap();
    assert!(bytes.windows(4).any(|w| w == b"Exif"));
    assert!(bytes.windows(4).any(|w| w == b"xml "));
    succeeds(
        cli()
            .arg(&input)
            .arg(&output)
            .args(["--force", "--lossless", "--strip-metadata"])
            .output()
            .unwrap(),
    );
    let bytes = fs::read(&output).unwrap();
    assert!(!bytes.windows(4).any(|w| w == b"Exif" || w == b"xml "));
    succeeds(
        cli()
            .arg(&input)
            .arg(&output)
            .args(["--force", "-q", "90"])
            .output()
            .unwrap(),
    );
    let bad = dir.path().join("bad.icc");
    fs::write(&bad, b"broken ICC").unwrap();
    assert!(
        !cli()
            .arg(&input)
            .arg(&output)
            .args(["--force", "--icc"])
            .arg(bad)
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[test]
fn apng_is_rejected_without_silently_encoding_one_frame() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("animated.png");
    let output = dir.path().join("output.jxl");
    let mut encoder = png::Encoder::new(fs::File::create(&input).unwrap(), 16, 16);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_animated(1, 0).unwrap();
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&samples(3, 8))
        .unwrap();
    let out = cli().arg(&input).arg(&output).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("APNG"));
    assert!(!output.exists());
}

#[test]
fn oversized_decoded_buffers_fail_before_allocation() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("huge.png");
    let output = dir.path().join("output.jxl");
    let mut encoder = png::Encoder::new(fs::File::create(&input).unwrap(), 20000, 20000);
    encoder.set_color(png::ColorType::Rgb);
    let mut writer = encoder.write_header().unwrap();
    writer.write_chunk(png::chunk::IDAT, &[0]).unwrap();
    drop(writer);
    let out = cli().arg(&input).arg(&output).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("allocation limit"));
    assert!(!output.exists());
}

#[test]
#[ignore = "requires djxl and jxlinfo from libjxl tools"]
fn independent_decoder_preserves_lossless_icc_and_metadata_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.png");
    let output = dir.path().join("output.jxl");
    let decoded = dir.path().join("decoded.png");
    let icc_file = dir.path().join("profile.icc");
    let profile = moxcms::ColorProfile::new_display_p3().encode().unwrap();
    let exif_data = exif(1);
    let raw = png(
        &input,
        4,
        16,
        Some(profile.clone()),
        Some(exif_data.clone()),
    );
    succeeds(
        cli()
            .arg(&input)
            .arg(&output)
            .arg("--lossless")
            .output()
            .unwrap(),
    );
    succeeds(
        Command::new("djxl")
            .arg(&output)
            .arg(&decoded)
            .arg(format!("--orig_icc_out={}", icc_file.display()))
            .output()
            .unwrap(),
    );
    assert_eq!(fs::read(&icc_file).unwrap(), profile);
    assert_eq!(decode_png(&decoded).1, raw);
    let extracted = dir.path().join("output.exif");
    djxl(&output, &extracted);
    assert_eq!(fs::read(&extracted).unwrap(), exif_data);
}

#[test]
#[ignore = "requires djxl and jxlinfo from libjxl tools"]
fn independent_decoder_checks_packed_gray_and_palette_transparency() {
    let dir = tempfile::tempdir().unwrap();
    for palette in [false, true] {
        let input = dir.path().join("input.png");
        let output = dir.path().join("output.jxl");
        let decoded = dir.path().join("decoded.png");
        let mut info = png::Info::with_size(16, 16);
        info.bit_depth = png::BitDepth::One;
        info.color_type = if palette {
            png::ColorType::Indexed
        } else {
            png::ColorType::Grayscale
        };
        if palette {
            info.palette = Some(Cow::Owned(vec![23, 45, 67, 89, 101, 123]));
            info.trns = Some(Cow::Owned(vec![0, 255]));
        }
        png::Encoder::with_info(fs::File::create(&input).unwrap(), info)
            .unwrap()
            .write_header()
            .unwrap()
            .write_image_data(&[0x55; 32])
            .unwrap();
        succeeds(
            cli()
                .arg(&input)
                .arg(&output)
                .args(["--lossless", "--force"])
                .output()
                .unwrap(),
        );
        djxl(&output, &decoded);
        let (info, pixels) = decode_png(&decoded);
        if palette {
            assert_eq!(info.color_type, png::ColorType::Rgba);
            for pair in pixels.as_chunks::<8>().0 {
                assert_eq!(pair, &[23, 45, 67, 0, 89, 101, 123, 255]);
            }
        } else {
            assert_eq!(info.color_type, png::ColorType::Grayscale);
            for pair in pixels.as_chunks::<2>().0 {
                assert_eq!(pair, &[0, 255]);
            }
        }
    }
}

#[test]
#[ignore = "requires djxl and jxlinfo from libjxl tools"]
fn independent_decoder_checks_orientation_metadata_and_hdr_signaling() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.png");
    let output = dir.path().join("output.jxl");
    let decoded = dir.path().join("decoded.png");
    let raw = png(&input, 3, 8, None, Some(exif(6)));
    for strip in [false, true] {
        let mut cmd = cli();
        cmd.arg(&input).arg(&output).args(["--lossless", "--force"]);
        if strip {
            cmd.arg("--strip-metadata");
        }
        succeeds(cmd.output().unwrap());
        djxl(&output, &decoded);
        let (_, pixels) = decode_png(&decoded);
        for y in 0..16 {
            for x in 0..16 {
                assert_eq!(
                    &pixels[(y * 16 + x) * 3..][..3],
                    &raw[((15 - x) * 16 + y) * 3..][..3]
                );
            }
        }
    }
    for transfer in [16, 18] {
        for primaries in [1, 9, 12] {
            let mut info = png::Info::with_size(16, 16);
            info.color_type = png::ColorType::Rgb;
            info.bit_depth = png::BitDepth::Sixteen;
            info.coding_independent_code_points = Some(png::CodingIndependentCodePoints {
                color_primaries: primaries,
                transfer_function: transfer,
                matrix_coefficients: 0,
                is_video_full_range_image: true,
            });
            let raw = samples(3, 16);
            let mut writer = png::Encoder::with_info(fs::File::create(&input).unwrap(), info)
                .unwrap()
                .write_header()
                .unwrap();
            writer
                .write_chunk(png::chunk::cICP, &[primaries, transfer, 0, 1])
                .unwrap();
            writer.write_image_data(&raw).unwrap();
            drop(writer);
            succeeds(
                cli()
                    .arg(&input)
                    .arg(&output)
                    .args(["--lossless", "--force"])
                    .output()
                    .unwrap(),
            );
            let info = succeeds(
                Command::new("jxlinfo")
                    .arg("-v")
                    .arg(&output)
                    .output()
                    .unwrap(),
            );
            let description = String::from_utf8_lossy(&info.stdout);
            assert!(
                description.contains(if transfer == 16 { "PQ" } else { "HLG" }),
                "{description}"
            );
            djxl(&output, &decoded);
            let (info, pixels) = decode_png(&decoded);
            assert_eq!(info.bit_depth, png::BitDepth::Sixteen);
            assert_eq!(pixels, raw);
        }
    }
}

#[test]
#[ignore = "requires djxl and jxlinfo from libjxl tools"]
fn independent_decoder_accepts_lossy_progressive_variants() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.png");
    let output = dir.path().join("output.jxl");
    let decoded = dir.path().join("decoded.png");
    png(&input, 3, 8, None, None);
    for flags in [
        vec!["--progressive"],
        vec!["--progressive-passes", "4"],
        vec!["--progressive-shifts", "2,1,0"],
        vec![
            "--progressive-dc",
            "--progressive-dc-levels",
            "2",
            "--group-order",
            "center-first",
        ],
    ] {
        succeeds(
            cli()
                .arg(&input)
                .arg(&output)
                .arg("--force")
                .args(flags)
                .output()
                .unwrap(),
        );
        djxl(&output, &decoded);
        let (info, _) = decode_png(&decoded);
        assert_eq!((info.width, info.height), (16, 16));
    }
}
