use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::signal;
use tracing::info;

/// Handle for graceful shutdown coordination.
#[derive(Debug, Clone)]
pub struct ShutdownHandle {
    shutting_down: Arc<AtomicBool>,
}

impl ShutdownHandle {
    pub fn new() -> Self {
        Self {
            shutting_down: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::Relaxed)
    }

    pub fn shutdown(&self) {
        info!("Shutdown signal received, initiating graceful shutdown");
        self.shutting_down.store(true, Ordering::Relaxed);
    }
}

impl Default for ShutdownHandle {
    fn default() -> Self {
        Self::new()
    }
}

/// Wait for shutdown signals (SIGTERM, SIGINT).
pub async fn wait_for_shutdown() {
    let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())
        .expect("Failed to install SIGTERM handler");
    let mut sigint = signal::unix::signal(signal::unix::SignalKind::interrupt())
        .expect("Failed to install SIGINT handler");

    tokio::select! {
        _ = sigterm.recv() => {
            info!("Received SIGTERM");
        }
        _ = sigint.recv() => {
            info!("Received SIGINT");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shutdown_handle() {
        let handle = ShutdownHandle::new();
        assert!(!handle.is_shutting_down());
        handle.shutdown();
        assert!(handle.is_shutting_down());
    }

    #[test]
    fn test_shutdown_handle_clone() {
        let handle = ShutdownHandle::new();
        let cloned = handle.clone();
        handle.shutdown();
        assert!(cloned.is_shutting_down());
    }
}
