use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn cli(input: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_jixel"));
    command.arg(input).args(["--threads", "4"]);
    command
}

fn success(output: Output) -> Output {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn png(path: &Path, width: u32, height: u32) {
    let mut encoder = png::Encoder::new(fs::File::create(path).unwrap(), width, height);
    encoder.set_color(png::ColorType::Rgb);
    let pixels: Vec<_> = (0..width * height * 3)
        .map(|i| (i * 137 + 17) as u8)
        .collect();
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&pixels)
        .unwrap();
}

fn jpeg(path: &Path, width: u32, height: u32) {
    image::codecs::jpeg::JpegEncoder::new(fs::File::create(path).unwrap())
        .encode(
            &vec![99; (width * height * 3) as usize],
            width,
            height,
            image::ExtendedColorType::Rgb8,
        )
        .unwrap();
}

#[test]
fn mixed_directory_detects_content_and_resumes_or_replaces_outputs() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("input");
    let output = temp.path().join("output");
    fs::create_dir(&input).unwrap();
    png(&input.join("пейзаж.unusual"), 16, 24);
    jpeg(&input.join("photo.JPG"), 24, 16);
    fs::write(input.join("notes.txt"), b"not an image").unwrap();
    let result = success(cli(&input).arg(&output).output().unwrap());
    assert!(String::from_utf8_lossy(&result.stderr).contains("2 successful, 1 skipped, 0 errors"));
    let encoded = output.join("photo.jxl");
    assert!(output.join("пейзаж.jxl").is_file());
    let original = fs::read(&encoded).unwrap();
    fs::write(&encoded, b"existing result").unwrap();
    let result = success(cli(&input).arg(&output).output().unwrap());
    assert!(String::from_utf8_lossy(&result.stderr).contains("0 successful, 3 skipped, 0 errors"));
    assert_eq!(fs::read(&encoded).unwrap(), b"existing result");
    success(cli(&input).arg(&output).arg("--force").output().unwrap());
    assert_eq!(fs::read(&encoded).unwrap(), original);
}

#[test]
fn recursion_preserves_structure_and_excludes_nested_output() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path();
    fs::create_dir(input.join("sub")).unwrap();
    png(&input.join("root.png"), 16, 16);
    png(&input.join("sub/root.png"), 16, 16);
    let output = input.join("encoded/deep");
    fs::create_dir_all(&output).unwrap();
    png(&output.join("do-not-process.png"), 16, 16);
    success(cli(input).arg(&output).output().unwrap());
    assert!(output.join("root.jxl").exists());
    assert!(!output.join("sub/root.jxl").exists());
    success(cli(input).arg(&output).arg("--recursive").output().unwrap());
    assert!(output.join("sub/root.jxl").exists());
    assert!(!output.join("encoded").exists());
    success(cli(input).arg("--recursive").output().unwrap());
    assert!(input.join("root.jxl").exists());
    assert!(input.join("sub/root.jxl").exists());
    success(cli(input).arg("--recursive").output().unwrap());
}

#[test]
fn longest_side_filter_has_an_inclusive_boundary_in_both_modes() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("input");
    fs::create_dir(&input).unwrap();
    png(&input.join("portrait.png"), 16, 24);
    png(&input.join("landscape.png"), 24, 16);
    jpeg(&input.join("photo.jpg"), 24, 16);
    for minimum in [23, 24, 25] {
        let output = temp.path().join(minimum.to_string());
        let result = success(
            cli(&input)
                .arg(&output)
                .args(["--min-long-edge", &minimum.to_string()])
                .output()
                .unwrap(),
        );
        if minimum <= 24 {
            for name in ["portrait", "landscape", "photo"] {
                assert!(output.join(format!("{name}.jxl")).is_file());
            }
        } else {
            assert!(!output.exists());
            assert!(
                String::from_utf8_lossy(&result.stderr)
                    .contains("0 successful, 3 skipped, 0 errors")
            );
        }
    }
    // The dimension gate must not allocate a pixel buffer for a skipped input.
    let huge = temp.path().join("huge");
    fs::create_dir(&huge).unwrap();
    let mut header = png::Encoder::new(
        fs::File::create(huge.join("header.png")).unwrap(),
        50_000,
        50_000,
    )
    .write_header()
    .unwrap();
    header.write_chunk(png::chunk::IDAT, &[0x78, 0x9c]).unwrap();
    header.finish().unwrap();
    success(
        cli(&huge)
            .args(["--min-long-edge", "50001"])
            .output()
            .unwrap(),
    );
}

#[test]
fn collisions_fail_before_writing_and_extension_template_resolves_them() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("input");
    let output = temp.path().join("output");
    fs::create_dir(&input).unwrap();
    png(&input.join("same.png"), 16, 16);
    jpeg(&input.join("same.jpg"), 16, 16);
    let result = cli(&input).arg(&output).arg("--force").output().unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("collision"));
    assert!(!output.exists());
    success(
        cli(&input)
            .arg(&output)
            .args(["--name", "{stem}-{ext}.jxl"])
            .output()
            .unwrap(),
    );
    assert!(output.join("same-png.jxl").exists());
    assert!(output.join("same-jpg.jxl").exists());
}

#[test]
fn broken_files_and_incompatible_modes_do_not_stop_other_tasks() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("input");
    let output = temp.path().join("output");
    fs::create_dir(&input).unwrap();
    fs::write(input.join("a-broken.jpg"), [0xff, 0xd8, 0xff, 0xe0]).unwrap();
    png(&input.join("b-valid.png"), 16, 16);
    jpeg(&input.join("c-valid.jpg"), 16, 16);
    let result = cli(&input)
        .arg(&output)
        .args(["--jobs", "2"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("a-broken.jpg"));
    assert!(stderr.contains("2 successful, 0 skipped, 1 errors"));
    assert!(output.join("b-valid.jxl").exists());
    assert!(output.join("c-valid.jxl").exists());
    let other = temp.path().join("other");
    let result = cli(&input)
        .arg(&other)
        .args(["--jpeg", "--quiet"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(other.join("c-valid.jxl").exists());
    assert!(!other.join("b-valid.jxl").exists());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("--jpeg requires JPEG input"));
    assert!(!stderr.contains("successful"));
}

#[test]
fn batch_options_are_validated_and_single_file_output_stays_required() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("image.png");
    png(&file, 16, 16);
    for options in [
        vec!["--jobs", "0"],
        vec!["--min-long-edge", "0"],
        vec!["--name", "../{stem}.jxl"],
        vec!["--name", "{index}.jxl"],
        vec!["--name", "{stem}.png"],
        vec!["-"],
    ] {
        let result = cli(temp.path()).args(&options).output().unwrap();
        assert_eq!(
            result.status.code(),
            Some(2),
            "{options:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!temp.path().join("image.jxl").exists());
    }
    assert_eq!(cli(&file).output().unwrap().status.code(), Some(2));
    for options in [
        vec!["--jobs", "2"],
        vec!["--recursive"],
        vec!["--min-long-edge", "10"],
        vec!["--name", "{stem}.jxl"],
    ] {
        let result = cli(&file)
            .arg(temp.path().join("image.jxl"))
            .args(options)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(2));
    }
}

#[test]
fn empty_and_quiet_batches_succeed() {
    let temp = tempfile::tempdir().unwrap();
    let result = success(cli(temp.path()).output().unwrap());
    assert!(String::from_utf8_lossy(&result.stderr).contains("0 successful, 0 skipped, 0 errors"));
    png(&temp.path().join("image.png"), 16, 16);
    let result = success(cli(temp.path()).arg("--quiet").output().unwrap());
    assert!(result.stderr.is_empty());
}

#[cfg(unix)]
#[test]
fn links_do_not_allow_overwriting_any_input_or_traversing_other_directories() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("input");
    let output = temp.path().join("output");
    fs::create_dir(&input).unwrap();
    fs::create_dir(&output).unwrap();
    png(&input.join("a.png"), 16, 16);
    jpeg(&input.join("b.jpg"), 16, 16);
    let original = fs::read(input.join("b.jpg")).unwrap();
    fs::hard_link(input.join("b.jpg"), output.join("a.jxl")).unwrap();
    let result = cli(&input).arg(&output).arg("--force").output().unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(!output.join("b.jxl").exists());
    assert_eq!(fs::read(input.join("b.jpg")).unwrap(), original);
    fs::remove_file(output.join("a.jxl")).unwrap();
    symlink(input.join("b.jpg"), output.join("a.jxl")).unwrap();
    assert_eq!(
        cli(&input)
            .arg(&output)
            .arg("--force")
            .output()
            .unwrap()
            .status
            .code(),
        Some(1)
    );
    fs::remove_file(output.join("a.jxl")).unwrap();
    symlink(&input, input.join("loop")).unwrap();
    symlink(input.join("a.png"), input.join("alias.png")).unwrap();
    success(
        cli(&input)
            .arg(&output)
            .arg("--recursive")
            .output()
            .unwrap(),
    );
    assert!(!output.join("loop").exists());
    assert!(!output.join("alias.jxl").exists());
    assert_eq!(fs::read(input.join("b.jpg")).unwrap(), original);
}

#[cfg(unix)]
#[test]
fn filenames_keep_non_utf8_bytes_and_same_path_sources_are_protected() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let temp = tempfile::tempdir().unwrap();
    let name = OsString::from_vec(b"photo-\xff.png".to_vec());
    png(&temp.path().join(name), 16, 16);
    success(
        cli(temp.path())
            .args(["--name", "{stem}-{ext}.jxl"])
            .output()
            .unwrap(),
    );
    assert!(
        temp.path()
            .join(OsString::from_vec(b"photo-\xff-png.jxl".to_vec()))
            .is_file()
    );
    let disguised = temp.path().join("original.jxl");
    png(&disguised, 16, 16);
    let before = fs::read(&disguised).unwrap();
    assert_eq!(
        cli(temp.path())
            .arg("--force")
            .output()
            .unwrap()
            .status
            .code(),
        Some(1)
    );
    assert_eq!(fs::read(disguised).unwrap(), before);
}

#[test]
#[ignore = "requires djxl from libjxl tools"]
fn independent_decoder_checks_batch_pixels_and_jpeg_reconstruction() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("input");
    let output = temp.path().join("output");
    fs::create_dir(&input).unwrap();
    png(&input.join("image.png"), 16, 24);
    jpeg(&input.join("photo.jpg"), 24, 16);
    success(cli(&input).arg(&output).output().unwrap());
    let restored = temp.path().join("restored.jpg");
    success(
        Command::new("djxl")
            .arg(output.join("photo.jxl"))
            .arg(&restored)
            .output()
            .unwrap(),
    );
    assert_eq!(
        fs::read(restored).unwrap(),
        fs::read(input.join("photo.jpg")).unwrap()
    );
    success(
        cli(&input)
            .arg(&output)
            .args(["--force", "--lossless"])
            .output()
            .unwrap(),
    );
    let decoded = temp.path().join("decoded.png");
    success(
        Command::new("djxl")
            .arg(output.join("image.jxl"))
            .arg(&decoded)
            .output()
            .unwrap(),
    );
    let read = |path: &Path| {
        let mut reader = png::Decoder::new(std::io::BufReader::new(fs::File::open(path).unwrap()))
            .read_info()
            .unwrap();
        let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut bytes).unwrap();
        bytes
    };
    assert_eq!(read(&decoded), read(&input.join("image.png")));
}

#[test]
fn blocked_output_subdirectory_does_not_stop_independent_inputs() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("input");
    let output = temp.path().join("output");
    fs::create_dir_all(input.join("sub")).unwrap();
    fs::create_dir(&output).unwrap();
    png(&input.join("root.png"), 16, 16);
    png(&input.join("sub/child.png"), 16, 16);
    fs::write(output.join("sub"), b"keep this file").unwrap();
    let result = cli(&input)
        .arg(&output)
        .arg("--recursive")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(output.join("root.jxl").is_file());
    assert_eq!(fs::read(output.join("sub")).unwrap(), b"keep this file");
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("sub/child.png"));
    assert!(stderr.contains("1 successful, 0 skipped, 1 errors"));
}

#[test]
fn directory_instead_of_output_is_an_error_even_with_force() {
    let temp = tempfile::tempdir().unwrap();
    for force in [false, true] {
        let input = temp.path().join(if force { "force" } else { "normal" });
        fs::create_dir_all(input.join("photo.jxl")).unwrap();
        png(&input.join("photo.png"), 16, 16);
        png(&input.join("other.png"), 16, 16);
        let mut command = cli(&input);
        if force {
            command.arg("--force");
        }
        let result = command.output().unwrap();
        assert_eq!(result.status.code(), Some(1));
        assert!(input.join("other.jxl").is_file());
        assert!(input.join("photo.jxl").is_dir());
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("1 successful, 0 skipped, 1 errors")
        );
    }
}

#[test]
fn output_collisions_follow_the_target_filesystem_rules() {
    // Set this to a writable directory on a case-insensitive volume to exercise that branch.
    let temp = match std::env::var_os("JIXEL_CASE_TEST_DIR") {
        Some(root) => tempfile::tempdir_in(root).unwrap(),
        None => tempfile::tempdir().unwrap(),
    };
    let input = temp.path().join("input");
    fs::create_dir(&input).unwrap();
    png(&input.join("Photo.jpg"), 16, 16);
    png(&input.join("photo.png"), 16, 16);
    let probe = temp.path().join("CaseProbe");
    fs::write(&probe, []).unwrap();
    let insensitive = temp.path().join("caseprobe").exists();
    fs::remove_file(probe).unwrap();
    for force in [false, true] {
        let output = temp.path().join(if force { "forced" } else { "normal" });
        let mut command = cli(&input);
        command.arg(&output);
        if force {
            command.arg("--force");
        }
        let result = command.output().unwrap();
        if insensitive {
            assert_eq!(result.status.code(), Some(1));
            assert!(String::from_utf8_lossy(&result.stderr).contains("collision"));
            assert!(!output.exists());
        } else {
            success(result);
            assert_eq!(fs::read_dir(&output).unwrap().count(), 2);
        }
    }
    assert_eq!(
        fs::read_dir(temp.path()).unwrap().count(),
        if insensitive { 1 } else { 3 }
    );
}
