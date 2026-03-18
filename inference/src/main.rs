pub mod config;
pub mod error;
pub mod model_loader;
pub mod model_registry;
pub mod request_queue;

/// In a full build, `tonic-build` overwrites `src/generated/sena.daemonbus.v1.rs`
/// from the proto definition. The placeholder file committed to the repo keeps
/// the crate compilable before the first `cargo build` runs codegen.
#[allow(dead_code)]
pub mod generated {
    #[path = "sena.daemonbus.v1.rs"]
    pub mod sena_daemonbus_v1;
}

fn main() {}
