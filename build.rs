use std::{
    fs,
    path::Path,
};

fn main() {
    let path = Path::new(&".").join("schema.sdl");
    fs::write(path, fuel_core_client::SCHEMA_SDL).unwrap();

    println!("cargo:rerun-if-changed=build.rs");
}
