#![forbid(unsafe_code)]

mod args;
mod batch;
mod input;

use args::Args;
use clap::{CommandFactory, FromArgMatches};
use std::{
    error::Error,
    fs,
    io::{self, IsTerminal, Read, Write},
    path::Path,
    process::ExitCode,
    time::Instant,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn check_output(args: &Args, output: &Path) -> Result<()> {
    let stdout = output == Path::new("-");
    if stdout && io::stdout().is_terminal() {
        return Err(
            "refusing to write binary JXL to a terminal; specify a file or redirect stdout".into(),
        );
    }
    let output_handle = if stdout {
        Some(same_file::Handle::stdout()?)
    } else {
        match same_file::Handle::from_path(output) {
            Ok(handle) => Some(handle),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        }
    };
    if let Some(output_handle) = output_handle {
        let input_handle = if args.input == Path::new("-") {
            same_file::Handle::stdin()?
        } else {
            same_file::Handle::from_path(&args.input)
                .map_err(|error| format!("cannot read input '{}': {error}", args.input.display()))?
        };
        if input_handle == output_handle {
            return Err("input and output refer to the same file".into());
        }
    }
    if !stdout && fs::symlink_metadata(output).is_ok() && !args.force {
        return Err("output already exists; use --force to replace it".into());
    }
    Ok(())
}

fn write_output(args: &Args, output: &Path, bytes: &[u8]) -> Result<()> {
    check_output(args, output)?;
    if output == Path::new("-") {
        let mut stdout = io::stdout().lock();
        stdout.write_all(bytes)?;
        stdout.flush()?;
    } else {
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        temp.write_all(bytes)?;
        temp.as_file().sync_all()?;
        if args.force {
            temp.persist(output)?;
        } else {
            temp.persist_noclobber(output)?;
        }
    }
    Ok(())
}

fn run_file(
    args: &Args,
    matches: &clap::ArgMatches,
    output: &Path,
    threads: usize,
    label: &Path,
    batch: bool,
) -> Result<bool> {
    let mut config = args.config()?;
    config.num_threads = threads;
    check_output(args, output)?;
    let mut bytes = Vec::new();
    if args.input == Path::new("-") {
        io::stdin().lock().read_to_end(&mut bytes)?;
    } else {
        bytes = fs::read(&args.input)
            .map_err(|error| format!("cannot read input '{}': {error}", args.input.display()))?;
    }
    let jpeg = input::is_jpeg(&bytes);
    if args.jpeg && !jpeg {
        return Err("--jpeg requires JPEG input".into());
    }
    let transcode = args.transcode(jpeg);
    args.validate_mode(matches, transcode)?;
    if let Some(minimum) = args.min_long_edge {
        let (width, height) = input::dimensions(&bytes)?;
        if width.max(height) < minimum {
            return Ok(false);
        }
    }
    let started = Instant::now();
    let encoded = if transcode {
        jixel::encode_jpeg_lossless_with_config(
            &bytes,
            &jixel::JpegTranscodeConfig::default()
                .with_speed(config.speed)
                .with_num_threads(config.num_threads)
                .with_jpeg_reconstruction(!args.no_jpeg_reconstruction),
        )?
    } else {
        let mut image = input::Input::decode(&bytes, args)?;
        image.configure(args, &mut config)?;
        image.encode(&config)?
    };
    if batch {
        fs::create_dir_all(output.parent().unwrap_or(Path::new(".")))?;
    }
    write_output(args, output, &encoded)
        .map_err(|error| format!("cannot write output '{}': {error}", output.display()))?;
    if !args.quiet {
        eprintln!(
            "jixel: {}: {} bytes -> {} bytes in {:.3}s ({})",
            label.display(),
            bytes.len(),
            encoded.len(),
            started.elapsed().as_secs_f64(),
            if transcode {
                "JPEG transcode"
            } else if args.lossless {
                if config.near_lossless > 0 {
                    "near-lossless pixels"
                } else {
                    "lossless pixels"
                }
            } else {
                "lossy pixels"
            }
        );
    }
    Ok(true)
}

fn run() -> Result<()> {
    let matches = Args::command().get_matches();
    let args = Args::from_arg_matches(&matches)?;
    let threads = args.config()?.num_threads;
    if args.input.is_dir() {
        return batch::run(&args, &matches, threads);
    }
    if args.recursive || args.jobs.is_some() || args.min_long_edge.is_some() || args.name.is_some()
    {
        return Err(args::invalid("batch options require an input directory").into());
    }
    let output = args
        .output
        .as_deref()
        .ok_or_else(|| args::invalid("an output file is required for a single input"))?;
    run_file(&args, &matches, output, threads, &args.input, false)?;
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if let Some(error) = error.downcast_ref::<clap::Error>() {
                let _ = error.print();
                ExitCode::from(2)
            } else {
                eprintln!("jixel: {error}");
                ExitCode::FAILURE
            }
        }
    }
}
