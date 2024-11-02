# INI Ninja

Get and set values from INI files while preserving the file's comments and formatting.
Get in and out of the file without being noticed 🥷

## Features

- Custom parsing logic written in pure rust, no slow regex found here.
- Can handle large files with low memory use, never needs to have the whole file in ram at once.
- Async and sync versions of read and write functions.
- Tests, CI, all the good things to make sure the code quality stays consistent in the future.
- No dependencies.

## Examples

#### Read value

```rust
use ini_ninja::IniParser;
fn main() {
    // Could also be a std::fs::File
    let ini_file = include_bytes!("../examples/ini_files/conan_exiles/DefaultGame.ini");

    // The default parser should work with most ini files
    let parser = IniParser::default();
    let max_players: Option<usize> = parser
        .read_value(ini_file.as_slice(), Some("/Script/Engine.GameSession"), "MaxPlayers")
        .unwrap();

    assert_eq!(max_players, Some(40));
}
```
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
