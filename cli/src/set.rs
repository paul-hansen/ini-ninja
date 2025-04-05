use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};

use ini_ninja::IniParser;
use tempfile::NamedTempFile;

static HELP_TEXT_SET: &str = "
Usage: ini-ninja[EXE] set [OPTIONS] <SECTION> <KEY> <VALUE> [File]

Arguments:
    <SECTION>  INI section the key is under.
               Use empty quotes for the global namespace.
               Don't include the square brackets.
    <KEY>      The key set the value for.
    [VALUE]    Value to set for the provided key.
    [FILE]     Path to the INI file to edit.

Options:
  -h, --help     Print help";

struct SetArgs<'a> {
    section: Option<&'a str>,
    key: &'a str,
    value: &'a str,
    path: &'a str,
}

impl<'a> SetArgs<'a> {
    fn parse(args: &'a [String]) -> SetArgs<'a> {
        let (section, key, value, file) = match args.len() {
            3 => (None, &args[0], &args[1], &args[2]),
            4 => (Some(&args[0]), &args[1], &args[2], &args[3]),
            x => {
                eprintln!("\"set\" expected 3 or 4 arguments, received {x} arguments.");
                std::process::exit(1);
            }
        };
        Self {
            section: section.map(|x| x.as_str()),
            key,
            value,
            path: file,
        }
    }
}
pub(crate) fn command_set(args: &[String]) {
    if args.is_empty() | ["-h", "--help"].contains(&args[0].as_str()) {
        println!("{HELP_TEXT_SET}");
        return;
    }
    let SetArgs {
        section,
        key,
        value,
        path,
    } = SetArgs::parse(args);
    let path = Path::new(path);
    let Ok(source) = File::open(path) else {
        eprintln!("Failed to open file at path: {}", path.to_string_lossy());
        std::process::exit(1);
    };
    let mut read_buffer = BufReader::new(source);
    let mut use_copy = false;

    // We'll initially write the changes to a temporary file and rename it to the original so it's
    // an atomic operation.
    let temp = if cfg!(target_os = "linux") {
        // Use the directory of the destination as temp dir to avoid
        // invalid cross-device link error when renaming,
        // and XDG_CACHE_DIR for fallback, failing that will fall back to using a copy instead of
        // rename.
        let xdg_cache = std::env::var("XDG_CACHE_DIR");
        let xdg_cache = xdg_cache.map(PathBuf::from).ok();
        let path = path.parent().or(xdg_cache.as_deref());
        if let Some(path) = path {
            NamedTempFile::new_in(path)
        } else {
            use_copy = true;
            NamedTempFile::new()
        }
    } else {
        NamedTempFile::new()
    };
    let temp = match temp {
        Ok(temp) => temp,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };

    let parser = IniParser::default();
    match parser.write_value(&mut read_buffer, &temp, section, key, value) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };
    // now we tell the OS to replace the original file with our modified version.
    if let Err(err) = if use_copy {
        std::fs::copy(temp.path(), path).map(|_| ())
    } else {
        std::fs::rename(temp.path(), path)
    } {
        eprintln!("Error while replacing original file with modified file: {err}");
        std::process::exit(1);
    }
    let _ = std::fs::remove_file(temp.path());
}
