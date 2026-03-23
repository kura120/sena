//! Build script for sena-ui Tauri backend — compiles daemon-bus proto definitions
//! into Rust client stubs.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Skip icon generation for scaffolding phase - icons will be added in Phase 3
    std::env::set_var("TAURI_SKIP_BUNDLE_CONFIG_VALIDATE", "true");

    tauri_build::build();

    println!("cargo:rerun-if-changed=../../daemon-bus/proto/sena.daemonbus.v1.proto");

    let proto_path = "../../daemon-bus/proto/sena.daemonbus.v1.proto";
    let proto_dir = "../../daemon-bus/proto";

    // Ensure the generated output directory exists.
    std::fs::create_dir_all("src/generated").ok();

    if protoc_available() {
        tonic_build::configure()
            .build_server(false)
            .build_client(true)
            .out_dir("src/generated")
            .compile_protos(&[proto_path], &[proto_dir])?;
    } else {
        println!("cargo:warning=protoc not found — using pre-committed generated stubs");
    }

    Ok(())
}

fn protoc_available() -> bool {
    if let Ok(path) = std::env::var("PROTOC") {
        return std::path::Path::new(&path).exists();
    }
    std::process::Command::new("protoc")
        .arg("--version")
        .output()
        .is_ok()
}
