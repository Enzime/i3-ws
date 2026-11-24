//! A minimal client for the i3 IPC protocol, which sway speaks too: a
//! 14-byte header (`"i3-ipc"`, payload length, message type; both integers
//! in native byte order) followed by the payload. Requests and replies use
//! the same framing.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::plan::{Output, Workspace};

const MAGIC: &[u8; 6] = b"i3-ipc";
const HEADER_LEN: usize = MAGIC.len() + 8;

const RUN_COMMAND: u32 = 0;
const GET_WORKSPACES: u32 = 1;
const GET_OUTPUTS: u32 = 3;

/// Finds the IPC socket of the running sway or i3: `$SWAYSOCK`, then
/// `$I3SOCK`, then `i3 --get-socketpath`.
pub fn socket_path() -> Result<PathBuf, String> {
    if let Some(path) = socket_from_env(std::env::var_os) {
        return Ok(path);
    }
    i3_socket_path().map_err(|e| {
        format!("no i3 or sway IPC socket found: SWAYSOCK and I3SOCK are unset and {e}")
    })
}

/// The socket named by the environment, if any. Empty values count as unset.
///
/// sway sets both variables to the same path for every process it starts.
/// i3 sets only `$I3SOCK`, and only for processes it starts with `exec`.
/// Only sway sets `$SWAYSOCK`, so it is checked first, like `swaymsg` does.
fn socket_from_env(var: impl Fn(&'static str) -> Option<OsString>) -> Option<PathBuf> {
    ["SWAYSOCK", "I3SOCK"]
        .into_iter()
        .filter_map(var)
        .find(|path| !path.is_empty())
        .map(PathBuf::from)
}

/// Asks i3 for its socket, which it reads from the X11 root window, so this
/// needs `$DISPLAY`. There is no sway equivalent: `sway --get-socketpath`
/// only prints `$SWAYSOCK`, which has already been checked.
fn i3_socket_path() -> Result<PathBuf, String> {
    let output = Command::new("i3")
        .arg("--get-socketpath")
        .output()
        .map_err(|e| format!("running `i3 --get-socketpath` failed: {e}"))?;
    let path = String::from_utf8_lossy(&output.stdout);
    let path = path.trim_end_matches('\n');
    if !output.status.success() || path.is_empty() {
        return Err(format!("`i3 --get-socketpath` failed ({})", output.status));
    }
    Ok(path.into())
}

/// One entry of a RUN_COMMAND reply.
#[derive(Debug, Deserialize)]
pub struct CommandOutcome {
    pub success: bool,
    #[serde(default)]
    pub error: Option<String>,
}

pub struct Connection {
    stream: UnixStream,
}

impl Connection {
    pub fn connect(path: &Path) -> Result<Self, String> {
        let stream = UnixStream::connect(path)
            .map_err(|e| format!("cannot connect to {}: {e}", path.display()))?;
        Ok(Self { stream })
    }

    pub fn get_outputs(&mut self) -> Result<Vec<Output>, String> {
        self.request("GET_OUTPUTS", GET_OUTPUTS, b"")
    }

    pub fn get_workspaces(&mut self) -> Result<Vec<Workspace>, String> {
        self.request("GET_WORKSPACES", GET_WORKSPACES, b"")
    }

    pub fn run_command(&mut self, command: &str) -> Result<Vec<CommandOutcome>, String> {
        self.request("RUN_COMMAND", RUN_COMMAND, command.as_bytes())
    }

    fn request<T: DeserializeOwned>(
        &mut self,
        name: &str,
        message_type: u32,
        payload: &[u8],
    ) -> Result<T, String> {
        let reply = self
            .exchange(message_type, payload)
            .map_err(|e| format!("{name} request failed: {e}"))?;
        serde_json::from_slice(&reply).map_err(|e| format!("invalid {name} reply: {e}"))
    }

    fn exchange(&mut self, message_type: u32, payload: &[u8]) -> std::io::Result<Vec<u8>> {
        let length =
            u32::try_from(payload.len()).map_err(|_| std::io::Error::other("payload too large"))?;
        let mut message = Vec::with_capacity(HEADER_LEN + payload.len());
        message.extend_from_slice(MAGIC);
        message.extend_from_slice(&length.to_ne_bytes());
        message.extend_from_slice(&message_type.to_ne_bytes());
        message.extend_from_slice(payload);
        self.stream.write_all(&message)?;

        let mut header = [0; HEADER_LEN];
        self.stream.read_exact(&mut header)?;
        if &header[..MAGIC.len()] != MAGIC {
            return Err(std::io::Error::other("reply has an invalid magic string"));
        }
        let field = |i: usize| u32::from_ne_bytes(header[i..i + 4].try_into().unwrap());
        let (length, reply_type) = (field(6), field(10));
        if reply_type != message_type {
            return Err(std::io::Error::other(format!(
                "expected a reply of type {message_type}, got {reply_type}"
            )));
        }

        let mut reply = vec![0; length as usize];
        self.stream.read_exact(&mut reply)?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let vars: Vec<(String, OsString)> = vars
            .iter()
            .map(|&(k, v)| (k.to_owned(), v.into()))
            .collect();
        move |name| vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
    }

    #[test]
    fn swaysock_takes_precedence_over_i3sock() {
        let var = env(&[("I3SOCK", "/run/i3"), ("SWAYSOCK", "/run/sway")]);
        assert_eq!(socket_from_env(var), Some("/run/sway".into()));
    }

    #[test]
    fn i3sock_is_used_without_swaysock() {
        let var = env(&[("I3SOCK", "/run/i3")]);
        assert_eq!(socket_from_env(var), Some("/run/i3".into()));
    }

    #[test]
    fn empty_variables_count_as_unset() {
        let var = env(&[("SWAYSOCK", ""), ("I3SOCK", "/run/i3")]);
        assert_eq!(socket_from_env(var), Some("/run/i3".into()));
        let var = env(&[("SWAYSOCK", ""), ("I3SOCK", "")]);
        assert_eq!(socket_from_env(var), None);
        assert_eq!(socket_from_env(env(&[])), None);
    }
}
