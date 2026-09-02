use super::error::RdpError;
use super::process::RdpProcess;
use super::types::RdpConnectionConfig;

/// Abstraction over "how do we launch a desktop" (ARCHITECTURE §5). Phase 4
/// ships a single implementation, `freeerdp::FreeRdpSidecarClient`; a future
/// embedded/WebRTC transport becomes a second. `close`/`status` are
/// deliberately NOT on this trait — they operate uniformly on the returned
/// `RdpProcess` handle and would not vary by transport.
pub trait RdpClient: Send + Sync {
    #[allow(async_fn_in_trait)] // native async trait; used with concrete types, not dyn
    async fn launch(&self, config: &RdpConnectionConfig) -> Result<RdpProcess, RdpError>;
}
