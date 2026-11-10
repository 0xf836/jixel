use crate::{
    Result,
    args::{Args, invalid},
    input,
};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    io::{self, Read},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

fn filename(template: &str, input: &Path) -> Result<OsString> {
    let mut name = OsString::new();
    let mut rest = template;
    while let Some(index) = rest.find(['{', '}']) {
        name.push(&rest[..index]);
        rest = &rest[index..];
        if let Some(tail) = rest.strip_prefix("{stem}") {
            name.push(input.file_stem().unwrap_or_default());
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix("{ext}") {
            name.push(input.extension().unwrap_or_default());
            rest = tail;
        } else {
            return Err(invalid("--name supports only {stem} and {ext}").into());
        }
    }
    name.push(rest);
    if template.contains(['/', '\\']) || !template.ends_with(".jxl") {
        return Err(
            invalid("--name must be one filename ending in .jxl, without separators").into(),
        );
    }
    Ok(name)
}

// Resolve existing parents too, so directory symlinks cannot hide collisions.
fn destination(path: &Path) -> Result<PathBuf> {
    match fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let mut resolved = destination(parent)?;
            match path.components().next_back() {
                Some(Component::Normal(name)) => resolved.push(name),
                Some(Component::ParentDir) => {
                    resolved.pop();
                }
                _ => return Err("invalid output path".into()),
            }
            Ok(resolved)
        }
        Err(error) => Err(error.into()),
    }
}

fn identity(path: &Path) -> Result<u64> {
    let mut hash = DefaultHasher::new();
    same_file::Handle::from_path(path)?.hash(&mut hash);
    // A hash collision only rejects a safe output; no open handles accumulate.
    Ok(hash.finish())
}

fn existing_output(path: &Path) -> Result<Option<u64>> {
    match fs::symlink_metadata(path) {
        Ok(_) if fs::metadata(path)?.is_file() => Ok(Some(identity(path)?)),
        Ok(_) => Err("output is not a regular file".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

#[derive(Default)]
struct Destinations {
    probes: HashMap<u64, tempfile::TempDir>,
    seen: HashMap<u64, PathBuf>,
}

impl Destinations {
    fn insert(
        &mut self,
        path: &Path,
        relative: &Path,
        existing: Option<u64>,
    ) -> Result<Option<PathBuf>> {
        let id = if let Some(id) = existing {
            id
        } else {
            let mut ancestor = path.parent().ok_or("output has no parent")?;
            loop {
                match fs::metadata(ancestor) {
                    Ok(metadata) if metadata.is_dir() => break,
                    Ok(_) => return Err("output parent is not a directory".into()),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        ancestor = ancestor.parent().ok_or("output has no existing parent")?;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            let probe = self.probes.entry(identity(ancestor)?);
            let directory = match probe {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(tempfile::tempdir_in(ancestor)?)
                }
            };
            // Let the target filesystem apply its own case and Unicode equivalence rules.
            let marker = directory.path().join(path.strip_prefix(ancestor)?);
            fs::create_dir_all(marker.parent().ok_or("probe has no parent")?)?;
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&marker)
            {
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists && marker.is_file() => {}
                Err(error) => return Err(error.into()),
            }
            identity(&marker)?
        };
        Ok(self.seen.insert(id, relative.to_owned()))
    }
}

fn discover(
    root: &Path,
    directory: &Path,
    excluded: Option<&Path>,
    recursive: bool,
    inputs: &mut Vec<PathBuf>,
    skipped: &mut usize,
    failed: &mut usize,
) -> Result<()> {
    let mut entries = fs::read_dir(directory)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let result = (|| -> Result<()> {
            let kind = entry.file_type()?;
            if kind.is_dir() {
                if recursive && excluded != Some(path.as_path()) {
                    discover(root, &path, excluded, recursive, inputs, skipped, failed)?;
                }
            } else if kind.is_file() {
                let mut header = Vec::with_capacity(8);
                fs::File::open(&path)?.take(8).read_to_end(&mut header)?;
                if input::is_jpeg(&header) || header.starts_with(b"\x89PNG\r\n\x1a\n") {
                    inputs.push(path.clone());
                } else {
                    *skipped += 1;
                }
            } else {
                *skipped += 1;
            }
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("jixel: {}: {error}", path.strip_prefix(root)?.display());
            *failed += 1;
        }
    }
    Ok(())
}

fn allocations(budget: usize, jobs: usize, count: usize) -> Vec<usize> {
    let workers = jobs.min(budget).min(count);
    if workers == 0 {
        return Vec::new();
    }
    (0..workers)
        .map(|i| budget / workers + usize::from(i < budget % workers))
        .collect()
}

fn dispatch<T: Sync>(items: &[T], allocations: &[usize], work: impl Fn(&T, usize) + Sync) {
    let cursor = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for &threads in allocations {
            let (cursor, work) = (&cursor, &work);
            scope.spawn(move || {
                while let Some(item) = items.get(cursor.fetch_add(1, Ordering::Relaxed)) {
                    work(item, threads);
                }
            });
        }
    });
}

pub fn run(args: &Args, matches: &clap::ArgMatches, budget: usize) -> Result<()> {
    if args.output.as_deref() == Some(Path::new("-")) {
        return Err(invalid("directory input requires a directory output, not stdout").into());
    }
    let template = args.name.as_deref().unwrap_or("{stem}.jxl");
    filename(template, Path::new("example.png"))?;
    let root = fs::canonicalize(&args.input)?;
    let output = destination(args.output.as_deref().unwrap_or(&root))?;
    if output.exists() && !output.is_dir() {
        return Err(invalid("directory input requires a directory output").into());
    }
    let excluded = (output != root && output.starts_with(&root)).then_some(output.as_path());
    let (mut inputs, mut skipped, mut failed) = (Vec::new(), 0, 0);
    discover(
        &root,
        &root,
        excluded,
        args.recursive,
        &mut inputs,
        &mut skipped,
        &mut failed,
    )?;
    let sources = inputs
        .iter()
        .map(|path| identity(path))
        .collect::<Result<HashSet<_>>>()?;
    let source_paths: HashSet<_> = inputs.iter().cloned().collect();
    let mut destinations = Destinations::default();
    let mut tasks = Vec::new();
    for input in inputs {
        let relative = input.strip_prefix(&root)?.to_owned();
        let path = output
            .join(relative.parent().unwrap_or(Path::new("")))
            .join(filename(template, &input)?);
        let prepared = (|| -> Result<_> { Ok((destination(&path)?, existing_output(&path)?)) })();
        let (resolved, existing) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                eprintln!("jixel: {}: {error}", relative.display());
                failed += 1;
                continue;
            }
        };
        if source_paths.contains(&resolved) || existing.is_some_and(|id| sources.contains(&id)) {
            return Err(format!(
                "output for '{}' refers to a batch input",
                relative.display()
            )
            .into());
        }
        let collision = match destinations.insert(&resolved, &relative, existing) {
            Ok(collision) => collision,
            Err(error) => {
                eprintln!("jixel: {}: {error}", relative.display());
                failed += 1;
                continue;
            }
        };
        if let Some(previous) = collision {
            return Err(format!(
                "output name collision: '{}' and '{}'; use --name '{{stem}}-{{ext}}.jxl'",
                previous.display(),
                relative.display()
            )
            .into());
        }
        tasks.push((input, path, relative));
    }
    drop(destinations);
    let allocations = allocations(budget, args.jobs.unwrap_or(4), tasks.len());
    let (successful, skipped_count, failed_count) = (
        AtomicUsize::new(0),
        AtomicUsize::new(skipped),
        AtomicUsize::new(failed),
    );
    dispatch(
        &tasks,
        &allocations,
        |(input, output, relative), threads| {
            let mut file_args = args.clone();
            file_args.input = input.clone();
            let result = (|| -> Result<bool> {
                if existing_output(output)?.is_some() && !args.force {
                    return Ok(false);
                }
                crate::run_file(&file_args, matches, output, threads, relative, true)
            })();
            match result {
                Ok(true) => {
                    successful.fetch_add(1, Ordering::Relaxed);
                }
                Ok(false) => {
                    skipped_count.fetch_add(1, Ordering::Relaxed);
                    if !args.quiet {
                        eprintln!("jixel: {}: skipped", relative.display());
                    }
                }
                Err(error) => {
                    failed_count.fetch_add(1, Ordering::Relaxed);
                    eprintln!("jixel: {}: {error}", relative.display());
                }
            }
        },
    );
    failed = failed_count.into_inner();
    if !args.quiet {
        eprintln!(
            "jixel: {} successful, {} skipped, {} errors",
            successful.into_inner(),
            skipped_count.into_inner(),
            failed
        );
    }
    if failed > 0 {
        return Err(format!("batch completed with {failed} errors").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Barrier, Mutex};

    #[test]
    fn worker_budget_is_bounded_and_each_task_runs_once() {
        assert_eq!(allocations(10, 4, 100), [3, 3, 2, 2]);
        assert_eq!(allocations(2, 4, 100), [1, 1]);
        assert_eq!(allocations(8, 4, 1), [8]);
        assert!(allocations(8, 4, 0).is_empty());
        let barrier = Barrier::new(4);
        let seen = Mutex::new(Vec::new());
        dispatch(
            &(0..32).collect::<Vec<_>>(),
            &[3, 3, 2, 2],
            |&item, threads| {
                if item < 4 {
                    barrier.wait();
                }
                seen.lock().unwrap().push((item, threads));
            },
        );
        let mut seen = seen.into_inner().unwrap();
        seen.sort_unstable();
        assert_eq!(
            seen.iter().map(|&(i, _)| i).collect::<Vec<_>>(),
            (0..32).collect::<Vec<_>>()
        );
        let mut first: Vec<_> = seen[..4].iter().map(|&(_, n)| n).collect();
        first.sort_unstable();
        assert_eq!(first, [2, 2, 3, 3]);
    }

    #[test]
    fn templates_are_small_and_preserve_names() {
        assert_eq!(
            filename("{stem}-{ext}.jxl", Path::new("a.b.PNG")).unwrap(),
            "a.b-PNG.jxl"
        );
        for bad in [
            "../{stem}.jxl",
            "a\\b.jxl",
            "{width}.jxl",
            "{stem.jxl",
            "a}.jxl",
            "{stem}.png",
        ] {
            assert!(filename(bad, Path::new("a.png")).is_err(), "{bad}");
        }
    }
}
