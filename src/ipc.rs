//! A minimal client for the i3 IPC protocol: a 14-byte header (`"i3-ipc"`,
//! payload length, message type; both integers in native byte order)
//! followed by the payload. Requests and replies use the same framing.

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

/// Finds the IPC socket: `$I3SOCK` if set, else `i3 --get-socketpath`.
pub fn socket_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("I3SOCK").filter(|p| !p.is_empty()) {
        return Ok(path.into());
    }

    let output = Command::new("i3")
        .arg("--get-socketpath")
        .output()
        .map_err(|e| format!("I3SOCK is not set and running `i3 --get-socketpath` failed: {e}"))?;
    let path = String::from_utf8_lossy(&output.stdout);
    let path = path.trim_end_matches('\n');
    if !output.status.success() || path.is_empty() {
        return Err(format!(
            "I3SOCK is not set and `i3 --get-socketpath` failed ({})",
            output.status
        ));
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
