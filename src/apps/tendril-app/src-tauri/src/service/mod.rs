pub mod agent_terminal_bridge;
pub mod changes_bridge;
pub mod client;
pub mod compatibility;
pub mod job_events_bridge;
pub mod master;
pub mod plan_mapping;
pub mod port_forward;
pub mod provision;
pub mod remote;
pub mod review_action_bridge;
pub mod supervisor;
pub mod tunnel_client;
pub mod ws_bridge;

/// The daemon sends a keep-alive on its event streams every 15 s. Three missed in a row means the
/// connection is dead even though the socket never said so (the network dropped under it), and the
/// stream reconnects instead of waiting for ever.
pub(crate) const STREAM_DEAD_AFTER: std::time::Duration = std::time::Duration::from_secs(45);
/// How long a stream's connect may take before it is given up and retried.
pub(crate) const STREAM_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

pub use changes_bridge::ChangeBridge;
pub use client::TendrilClient;
pub use compatibility::{SemVer, ServiceCompatibilityManager, VersionCheckResult};
pub use master::MasterDiscovery;
pub use plan_mapping::{map_plan_detail, map_plan_summary};
// The platform layer moved to `tendril-core` so `tendril service install` and the app write the
// same unit; re-exported here under its old path because the app's callers and tests name it.
pub use provision::{provision, AutostartOutcome, ProvisionReport};
pub use supervisor::{
    calculate_backoff_secs, redact_sensitive_tokens, CircuitBreaker, ServiceOwnership,
    ServiceSupervisor, SupervisorStateInfo, SupervisorStatus,
};
pub use tendril_core::service::platform;
pub use tendril_core::service::PlatformServiceConfig;
pub use tunnel_client::{CloudflaredInstallDto, TunnelClient, TunnelSnapshotDto};
pub use ws_bridge::WsBridge;
