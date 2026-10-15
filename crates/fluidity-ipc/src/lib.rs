mod client;
mod framing;
mod protocol;

pub use client::{Client, Error};
pub use framing::{read_message, write_message};
pub use protocol::*;

#[cfg(unix)]
pub const SOCKET_PATH: &str = "/var/run/fluidityd.sock";
#[cfg(windows)]
pub const PIPE_NAME: &str = r"\\.\pipe\fluidityd";

pub fn socket_path() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("FLUIDITY_SOCKET") {
        return path.into();
    }
    #[cfg(unix)]
    return SOCKET_PATH.into();
    #[cfg(windows)]
    return PIPE_NAME.into();
}
