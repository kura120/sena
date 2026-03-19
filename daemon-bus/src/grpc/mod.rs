//! gRPC server module for daemon-bus.
//!
//! Exposes all daemon-bus services over gRPC so child subsystems can:
//! - Signal boot readiness via `BootService.SignalReady`
//! - Query boot status via `BootService.GetBootStatus`
//! - (Future: subscribe to event bus, request priority escalation, etc.)
//!
//! The gRPC server runs in a background tokio task spawned by `start_grpc_server()`.
//! If the server fails to bind or crashes, it logs the error with structured
//! tracing but does not panic — daemon-bus enters degraded mode and continues
//! operating so diagnostics can be collected.

pub mod boot_service;

use std::net::SocketAddr;

use tonic::transport::Server;
use tokio_stream::wrappers::TcpListenerStream;

use crate::boot::BootOrchestrator;
use crate::config::GrpcConfig;
use crate::error::{ErrorCode, SenaError, SenaResult};
use crate::generated::sena_daemonbus_v1::boot_service_server::BootServiceServer;

use self::boot_service::BootServiceHandler;

/// Start the gRPC server and return only after the TCP socket is bound.
///
/// Binds the TCP listener synchronously (before spawning the serve task) so
/// that the OS port is reserved and child subsystems can connect as soon as
/// this function returns — there is no race between "task spawned" and "port
/// open".
///
/// `serve_with_incoming` is used instead of `serve` so the already-bound
/// listener is handed directly to tonic; the port is never rebound inside the
/// spawned task.
///
/// If the bind or address parse fails, this returns an error immediately.
/// If the serve loop subsequently crashes, the error is logged and daemon-bus
/// continues in degraded mode — it does not panic.
///
/// Returns a `JoinHandle<()>` that can be awaited for shutdown or monitoring.
pub async fn start_grpc_server(
    grpc_config: &GrpcConfig,
    boot_orchestrator: BootOrchestrator,
) -> SenaResult<tokio::task::JoinHandle<()>> {
    // Parse the socket address from config.
    let addr_str = grpc_config.socket_addr();
    let addr: SocketAddr = addr_str.parse().map_err(|parse_err| {
        SenaError::new(
            ErrorCode::GrpcServerFailed,
            format!("failed to parse gRPC bind address: {}", parse_err),
        )
        .with_debug_context(format!("addr_str = '{}'", addr_str))
    })?;

    // Bind the TCP socket before spawning the task. After this await the OS
    // has reserved the port — child subsystems can connect as soon as we
    // return from start_grpc_server, with no observable window where the
    // port is not yet open.
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|bind_err| {
        SenaError::new(
            ErrorCode::GrpcServerFailed,
            format!("failed to bind gRPC server to {}: {}", addr, bind_err),
        )
        .with_debug_context(format!("addr_str = '{}'", addr_str))
    })?;

    // Read back the actual bound address (may differ from `addr` when port 0
    // was requested for ephemeral port assignment in tests).
    let bound_addr = listener.local_addr().map_err(|addr_err| {
        SenaError::new(
            ErrorCode::GrpcServerFailed,
            format!("failed to read bound local address: {}", addr_err),
        )
    })?;

    tracing::info!(
        subsystem = "daemon_bus",
        event_type = "grpc_server_bound",
        bind_address = %bound_addr,
        "gRPC server bound — spawning serve task"
    );

    // Create the BootService handler and wrap it in the generated server.
    let boot_service_handler = BootServiceHandler::new(boot_orchestrator);
    let boot_service = BootServiceServer::new(boot_service_handler);

    // Wrap the already-bound listener in a stream that tonic can consume.
    // TcpListenerStream implements Stream<Item = Result<TcpStream, io::Error>>,
    // which satisfies serve_with_incoming's bound.
    let incoming = TcpListenerStream::new(listener);

    // Spawn the serve loop. The port is already bound, so connections arriving
    // before the task is scheduled will be queued by the OS backlog.
    let handle = tokio::spawn(async move {
        let server_result = Server::builder()
            .add_service(boot_service)
            .serve_with_incoming(incoming)
            .await;

        // If serve_with_incoming returns an error, log it with full context.
        // Do not panic — allow daemon-bus to continue in degraded mode.
        if let Err(serve_err) = server_result {
            tracing::error!(
                subsystem = "daemon_bus",
                event_type = "grpc_server_failed",
                bind_address = %bound_addr,
                error = %serve_err,
                "gRPC server exited with error"
            );
        } else {
            // Graceful shutdown — server was terminated cleanly.
            tracing::info!(
                subsystem = "daemon_bus",
                event_type = "grpc_server_stopped",
                bind_address = %bound_addr,
                "gRPC server stopped gracefully"
            );
        }
    });

    tracing::info!(
        subsystem = "daemon_bus",
        event_type = "grpc_server_started",
        bind_address = %bound_addr,
        "gRPC server bound and accepting connections"
    );

    Ok(handle)
}
