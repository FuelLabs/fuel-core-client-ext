use std::fs;

fn main() {
    fs::write("target/schema.sdl", fuel_core_client::SCHEMA_SDL)
        .expect("Unable to write schema file");

    println!("cargo:rerun-if-changed=build.rs");
}
