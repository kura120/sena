

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // tonic-build compiles .proto files into Rust code at build time.
    // Output goes to src/generated/ — never edited manually.
    //
    // If protoc is not installed, we skip codegen gracefully. The pre-committed
    // placeholder file in src/generated/sena.daemonbus.v1.rs keeps the crate
    // compilable without protoc for initial scaffolding and CI environments
    // that haven't installed the protobuf compiler yet.
    //
    // Once protoc is available, this build script overwrites the placeholder
    // with the real generated code.

    // Re-run codegen whenever the proto file changes.
    println!("cargo:rerun-if-changed=proto/sena.daemonbus.v1.proto");
    
    let protoc_available = which_protoc().is_some();

    if !protoc_available {
        println!(
            "cargo:warning=protoc not found — skipping proto codegen. \
             Using pre-committed placeholder in src/generated/. \
             Install protoc to regenerate from proto definitions."
        );
        return Ok(());
    }

    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .out_dir("src/generated")
        .compile_protos(&["proto/sena.daemonbus.v1.proto"], &["proto"])?;

    Ok(())
}

/// Check whether protoc is discoverable — either via the PROTOC env var or on PATH.
/// Returns the path if found, None otherwise.
fn which_protoc() -> Option<std::path::PathBuf> {
    // First check the PROTOC environment variable (standard prost-build convention).
    if let Ok(protoc_path) = std::env::var("PROTOC") {
        let path = std::path::PathBuf::from(&protoc_path);
        if path.exists() {
            return Some(path);
        }
    }

    // Fall back to searching PATH for the protoc binary.
    let protoc_name = if cfg!(windows) {
        "protoc.exe"
    } else {
        "protoc"
    };

    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(protoc_name))
            .find(|candidate| candidate.is_file())
    })
}
