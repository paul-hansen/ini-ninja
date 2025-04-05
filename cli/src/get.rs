use std::fs::File;

use ini_ninja::IniParser;

static HELP_TEXT_GET: &str = "
Usage: ini-ninja[EXE] get [OPTIONS] <SECTION> <KEY> [File]

Arguments:
    <SECTION>  INI section the key is under.
               Use empty quotes for the global namespace.
               Don't include the square brackets.
    <KEY>      The key to retrieve the value for.

Options:
  -h, --help     Print help";

struct GetArgs<'a> {
    section: Option<&'a str>,
    key: &'a str,
    path: &'a str,
}

impl<'a> GetArgs<'a> {
    fn parse(args: &'a [String]) -> GetArgs<'a> {
        let (section, key, file) = match args.len() {
            2 => (None, &args[0], &args[1]),
            3 => (Some(&args[0]), &args[1], &args[2]),
            x => {
                eprintln!("\"get\" expected 2 or 3 arguments, received {x} arguments.");
                std::process::exit(1);
            }
        };
        Self {
            section: section.map(|x| x.as_str()),
            key,
            path: file,
        }
    }
}

pub(crate) fn command_get(args: &[String]) {
    if args.is_empty() | ["-h", "--help"].contains(&args[0].as_str()) {
        println!("{HELP_TEXT_GET}");
        return;
    }
    let GetArgs { section, key, path } = GetArgs::parse(args);
    let parser = IniParser::default();
    let Ok(source) = File::open(path) else {
        eprintln!("Failed to open file at path: {path}");
        std::process::exit(1);
    };
    let value = match parser.read_value::<String>(source, section, key) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };
    if let Some(value) = value {
        println!("{value}");
    }
}
