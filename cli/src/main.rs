use get::command_get;
use set::command_set;
mod get;
mod set;

static HELP_TEXT: &str = "
Get or set values in INI files while preserving formatting & comments.

Usage: ini-ninja[EXE] [OPTIONS] <COMMAND> [ARGUMENTS]

Commands:
    get <section> <key>          Get a value from an ini file
    set <section> <key> <value>  Set a value in the ini file

Options:
  -h, --help     Print help
  -V, --version  Print version";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|x| x.as_str()) {
        Some("get") => command_get(&args[2..]),
        Some("set") => command_set(&args[2..]),
        Some("-h") | Some("--help") | None => println!("{HELP_TEXT}"),
        Some("-V") | Some("--version") => {
            println!("{}", std::env!("CARGO_PKG_VERSION"))
        }
        Some(x) => {
            eprintln!("Unknown parameter \"{x}\"")
        }
    }
}
