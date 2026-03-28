//! Build script for memory-engine — compiles daemon-bus proto definitions into
//! Rust client and server stubs. memory-engine acts as both:
//!
//! - **Client** — connects to daemon-bus (BootService, EventBusService)
//! - **Server** — serves MemoryService to other subsystems via daemon-bus routing
//!
//! If protoc is not installed, codegen is skipped and the pre-committed
//! placeholder in src/generated/ keeps the crate compilable.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Re-run codegen whenever the proto file changes.
    println!("cargo:rerun-if-changed=../shared/proto/sena.daemonbus.v1.proto");

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
        .compile_protos(
            &["../shared/proto/sena.daemonbus.v1.proto"],
            &["../shared/proto"],
        )?;

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
