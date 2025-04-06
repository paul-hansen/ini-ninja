# INI Ninja

Get and set values from INI files while preserving the file's comments and formatting.
Get in and out of the file without being noticed 🥷


## Features

- CLI and Rust crate.
- Custom parsing logic written in rust, no slow regex.
- Can handle large files with low memory use, never needs to have the whole file in memory at once.
- Async and sync versions of read and write functions.
- Tests, CI, all the good things to make sure the code quality stays consistent in the future.
- No dependencies. (fast to build, no bloat, CLI app is tiny)


## CLI

### Install
```text,ignore
cargo install --git=https://github.com/paul-hansen/ini-ninja/cli
```

### Usage

```text,ignore
ini-ninja -h
ini-ninja get -h
ini-ninja set -h
```

```text,ignore
ini-ninja get section key ./some_path
```


## Library Examples

#### Read value from file

```rust
use ini_ninja::IniParser;
fn main() -> Result<(), Box<dyn std::error::Error>>{
    let ini_file = std::fs::File::open("../examples/ini_files/simple.ini")?;

    // The default parser should work with most ini files
    let parser = IniParser::default();
    let max_players: Option<String> = parser
        .read_value(ini_file, Some("user"), "first_name")
        .unwrap();

    assert_eq!(max_players, Some("Bob".to_string()));
    Ok(())
}
```

#### Read value from String

```rust
use ini_ninja::IniParser;
fn main() -> Result<(), Box<dyn std::error::Error>>{
    let ini_file: String = include_str!("examples/ini_files/simple.ini").to_string();

    // The default parser should work with most ini files
    let parser = IniParser::default();
    let max_players: Option<String> = parser
        .read_value(ini_file.as_bytes(), Some("user"), "first_name")
        .unwrap();

    assert_eq!(max_players, Some("Bob".to_string()));
    Ok(())
}
```

## Drawbacks

This crate will scan the file's contents every time you read or write a value. If you are reading/writing many values, this may not be the most efficient for your use case.

## Comparison with other Rust crates

- [ini-roudtrip](https://github.com/VorpalBlade/ini-roundtrip) - Preserves formatting and comments but inserting and writing to a file is left to the user.
- [configparser](https://github.com/QEDK/configparser-rs) Does not preserve comments [configparser-rs#5](https://github.com/QEDK/configparser-rs/issues/5)
- [rust-ini](https://github.com/zonyitoo/rust-ini) - Does not preserve comments [rust-ini](https://github.com/zonyitoo/rust-ini/issues/77)
- [pretty_ini](https://github.com/eVisualUser/pretty-ini) - Does not preserve formatting
- [ini_core](https://github.com/CasualX/ini_core) - No writing

## License

Licensed under either of

 * Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
 * MIT license
   ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
