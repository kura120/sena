pub mod app;
pub mod components;
pub mod config;
pub mod debug_state;
pub mod grpc;
pub mod state;
pub mod theme;

pub mod proto {
    include!("generated/sena.daemonbus.v1.rs");
}
