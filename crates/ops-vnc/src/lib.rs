//! Native RFB (VNC) client replacing the original noVNC + WebSocket bridge (TASKS M8-10).

pub mod auth;
pub mod client;
pub mod config;
pub mod keys;
pub mod session;

pub use config::{load_vnc_host_config, VncHostConfig, DEFAULT_VNC_PORT};
pub use session::{start_session, VncCommand, VncEvent, VncSession};

#[cfg(test)]
mod tests;
