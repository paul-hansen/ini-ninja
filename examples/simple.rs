use std::{error::Error, fs::File};

fn main() -> Result<(), Box<dyn Error>> {
    let path = "./examples/ini_files/simple.ini";
    let section = Some("user");
    let key = "first_name";
    let new_value = "John";
    let parser = ini_ninja::IniParser::default();
    let mut file = File::open(path)?;
    let value = parser.read_value::<String>(&file, section, key)?;
    if let Some(value) = value {
        println!("Original value was: {value}");
    } else {
        return Err("Value not found".into());
    }

    let mut output = Vec::new();
    parser.write_value(&mut file, &mut output, section, key, new_value)?;
    let output = String::from_utf8(output)?;
    let new_value = parser.read_value::<String>(output.as_bytes(), section, key)?;
    if let Some(new_value) = new_value {
        println!("New value was: {new_value}");
    } else {
        return Err("Value not found".into());
    }
    Ok(())
}
