# INI Ninja

Get and set values from INI files while preserving the file's comments and formatting.
Get in and out of the file without being noticed 🥷

## Features

- Custom parsing logic written in rust, no slow regex.
- Can handle large files with low memory use, never needs to have the whole file in memory at once.
- Async and sync versions of read and write functions.
- Tests, CI, all the good things to make sure the code quality stays consistent in the future.
- No dependencies.

## Examples

#### Read value

```rust
use ini_ninja::IniParser;
fn main() {
    // Could also be a std::fs::File
    let ini_file = include_bytes!("../examples/ini_files/simple.ini");

    // The default parser should work with most ini files
    let parser = IniParser::default();
    let max_players: Option<String> = parser
        .read_value(ini_file.as_slice(), Some("user"), "first_name")
        .unwrap();

    assert_eq!(max_players, Some("Bob".to_string()));
}
```

## Drawbacks

This crate will scan the file's contents every time you read or write a value. If you are reading/writing a majority of the values, this may not be the most efficient for your use case.
This allows it to ensure the rest of the file's contents are untouched,
by scanning the file for the start and end location of the value, and using that to replace it with the new value.

## Other Crates
- [configparser](https://github.com/QEDK/configparser-rs) Does not preserve comments [configparser-rs#5](https://github.com/QEDK/configparser-rs/issues/5)
- [rust-ini](https://github.com/zonyitoo/rust-ini) - Does not preserve comments [rust-ini](https://github.com/zonyitoo/rust-ini/issues/77)

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
