//! Runs the real binary against a fake window manager listening on a Unix
//! socket and checks the exact IPC messages it sends.

use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};

use serde_json::{Value, json};

const RUN_COMMAND: u32 = 0;
const GET_WORKSPACES: u32 = 1;
const GET_OUTPUTS: u32 = 3;

/// A tiny model of a window manager: outputs and the workspaces on them.
#[derive(Clone)]
struct FakeWm {
    /// (name, x, visible workspace)
    outputs: Vec<(&'static str, i64, String)>,
    /// (name, output)
    workspaces: Vec<(String, &'static str)>,
    focused: String,
    /// Reply to `move` commands like i3 does on an empty workspace.
    reject_moves: bool,
}

impl FakeWm {
    fn new(outputs: &[(&'static str, i64, &str)], focused: &str) -> Self {
        Self {
            outputs: outputs
                .iter()
                .map(|&(name, x, ws)| (name, x, ws.to_owned()))
                .collect(),
            workspaces: outputs
                .iter()
                .map(|&(name, _, ws)| (ws.to_owned(), name))
                .collect(),
            focused: focused.to_owned(),
            reject_moves: false,
        }
    }

    fn with_workspace(mut self, name: &str, output: &'static str) -> Self {
        self.workspaces.push((name.to_owned(), output));
        self
    }

    fn focused_output(&self) -> &'static str {
        self.workspaces
            .iter()
            .find(|(name, _)| *name == self.focused)
            .map_or("", |&(_, output)| output)
    }

    fn outputs_json(&self) -> Value {
        let mut outputs = vec![json!({
            "name": "xroot-0",
            "active": false,
            "primary": false,
            "rect": {"x": 0, "y": 0, "width": 3840, "height": 1080},
            "current_workspace": null,
        })];
        outputs.extend(self.outputs.iter().map(|(name, x, ws)| {
            json!({
                "name": name,
                "active": true,
                "primary": false,
                "rect": {"x": x, "y": 0, "width": 1920, "height": 1080},
                "current_workspace": ws,
            })
        }));
        Value::Array(outputs)
    }

    fn workspaces_json(&self) -> Value {
        self.workspaces
            .iter()
            .map(|(name, output)| {
                json!({
                    "num": name.parse::<i64>().unwrap_or(-1),
                    "name": name,
                    "visible": self.outputs.iter().any(|(_, _, ws)| ws == name),
                    "focused": *name == self.focused,
                    "urgent": false,
                    "rect": {"x": 0, "y": 0, "width": 1920, "height": 1080},
                    "output": output,
                })
            })
            .collect()
    }

    fn run_command(&mut self, command: &str) -> Value {
        if let Some(name) = command.strip_prefix("workspace ") {
            let output = match self.workspaces.iter().find(|(ws, _)| ws == name) {
                Some(&(_, output)) => output,
                None => {
                    let output = self.focused_output();
                    self.workspaces.push((name.to_owned(), output));
                    output
                }
            };
            for (out, _, ws) in &mut self.outputs {
                if *out == output {
                    *ws = name.to_owned();
                }
            }
            self.focused = name.to_owned();
            json!([{"success": true}])
        } else if let Some(name) = command.strip_prefix("move window to workspace ") {
            if self.reject_moves {
                return json!([{"success": false, "error": "Nothing to move: workspace empty"}]);
            }
            if !self.workspaces.iter().any(|(ws, _)| ws == name) {
                let output = self.focused_output();
                self.workspaces.push((name.to_owned(), output));
            }
            json!([{"success": true}])
        } else {
            json!([{"success": false, "parse_error": true, "error": "unknown command"}])
        }
    }
}

/// Everything the client sent: (message type, payload) in order.
type Requests = Vec<(u32, String)>;

fn socket_path() -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "i3-ws-test-{}-{}.sock",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Serves one client connection until it disconnects.
fn serve(listener: UnixListener, mut wm: FakeWm) -> Requests {
    let (mut stream, _) = listener.accept().unwrap();
    let mut requests = Vec::new();
    loop {
        let mut header = [0; 14];
        match stream.read_exact(&mut header) {
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => return requests,
            result => result.unwrap(),
        }
        assert_eq!(&header[..6], b"i3-ipc");
        let length = u32::from_ne_bytes(header[6..10].try_into().unwrap());
        let message_type = u32::from_ne_bytes(header[10..14].try_into().unwrap());
        let mut payload = vec![0; length as usize];
        stream.read_exact(&mut payload).unwrap();
        let payload = String::from_utf8(payload).unwrap();

        let reply = match message_type {
            GET_OUTPUTS => wm.outputs_json(),
            GET_WORKSPACES => wm.workspaces_json(),
            RUN_COMMAND => wm.run_command(&payload),
            other => panic!("unexpected message type {other}"),
        };
        requests.push((message_type, payload));

        let reply = serde_json::to_vec(&reply).unwrap();
        let mut message = b"i3-ipc".to_vec();
        message.extend_from_slice(&(reply.len() as u32).to_ne_bytes());
        message.extend_from_slice(&message_type.to_ne_bytes());
        message.extend_from_slice(&reply);
        stream.write_all(&message).unwrap();
    }
}

struct Run {
    output: Output,
    requests: Requests,
}

impl Run {
    fn commands(&self) -> Vec<&str> {
        self.requests
            .iter()
            .filter(|(t, _)| *t == RUN_COMMAND)
            .map(|(_, p)| p.as_str())
            .collect()
    }

    fn types(&self) -> Vec<u32> {
        self.requests.iter().map(|&(t, _)| t).collect()
    }

    fn stdout(&self) -> &str {
        std::str::from_utf8(&self.output.stdout).unwrap()
    }

    fn stderr(&self) -> &str {
        std::str::from_utf8(&self.output.stderr).unwrap()
    }
}

fn i3_ws(socket: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_i3-ws"))
        .args(args)
        .env("I3SOCK", socket)
        .env_remove("SWAYSOCK")
        .output()
        .unwrap()
}

fn run(wm: FakeWm, args: &[&str]) -> Run {
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).unwrap();
    let server: JoinHandle<Requests> = thread::spawn(move || serve(listener, wm));

    let output = i3_ws(&path, args);
    // Unblock the server if the binary never connected.
    drop(UnixStream::connect(&path));
    let requests = server.join().unwrap();
    std::fs::remove_file(&path).unwrap();
    Run { output, requests }
}

fn two_outputs(focused: &str) -> FakeWm {
    // Listed right to left; the binary must sort them by position.
    FakeWm::new(&[("DP-2", 1920, "205"), ("HDMI-1", 0, "101")], focused)
        .with_workspace("103", "HDMI-1")
        .with_workspace("207", "DP-2")
}

#[test]
fn switches_to_the_next_output() {
    let run = run(two_outputs("101"), &["next"]);
    assert!(run.output.status.success(), "{}", run.stderr());
    assert_eq!(
        run.requests,
        [
            (GET_OUTPUTS, String::new()),
            (GET_WORKSPACES, String::new()),
            (RUN_COMMAND, "workspace 205".to_owned()),
        ]
    );
    assert_eq!(run.stdout(), "1\nDP-2 205\n");
}

#[test]
fn shifts_a_window_to_an_output_and_follows_it() {
    let run = run(two_outputs("205"), &["--shift", "1"]);
    assert!(run.output.status.success(), "{}", run.stderr());
    assert_eq!(
        run.commands(),
        ["move window to workspace 101", "workspace 101"]
    );
}

#[test]
fn creates_the_next_workspace_on_the_focused_output() {
    let run = run(two_outputs("205"), &["--ws", "--create", "next"]);
    assert!(run.output.status.success(), "{}", run.stderr());
    assert_eq!(run.commands(), ["workspace 206"]);
    assert_eq!(run.stdout(), "205\n");
}

#[test]
fn walks_existing_workspaces_with_loop() {
    let run = run(two_outputs("207"), &["--ws", "--loop", "--shift", "next"]);
    assert!(run.output.status.success(), "{}", run.stderr());
    assert_eq!(run.commands(), ["move window to workspace 205"]);
}

#[test]
fn sends_nothing_when_already_there() {
    let same = run(two_outputs("205"), &["--ws", "5"]);
    assert!(same.output.status.success(), "{}", same.stderr());
    assert_eq!(same.types(), [GET_OUTPUTS, GET_WORKSPACES]);

    let edge = run(two_outputs("207"), &["--ws", "next"]);
    assert!(edge.output.status.success(), "{}", edge.stderr());
    assert_eq!(edge.types(), [GET_OUTPUTS, GET_WORKSPACES]);
    assert_eq!(edge.stdout(), "207\nAt the last workspace, use --loop\n");
}

#[test]
fn init_rereads_the_focused_workspace_for_each_output() {
    let wm = FakeWm::new(&[("DP-2", 1920, "7"), ("HDMI-1", 0, "3")], "3");
    let run = run(wm, &["--init"]);
    assert!(run.output.status.success(), "{}", run.stderr());
    assert_eq!(
        run.types(),
        [
            GET_OUTPUTS,
            GET_WORKSPACES,
            GET_WORKSPACES,
            RUN_COMMAND,
            GET_WORKSPACES,
            RUN_COMMAND,
            RUN_COMMAND,
        ]
    );
    assert_eq!(
        run.commands(),
        ["workspace 101", "workspace 7", "workspace 201"]
    );
    assert_eq!(
        run.stdout(),
        "101 HDMI-1\nSwitching to 101\n\
         201 DP-2\nSwitching from 101\nSwitching to 7\nSwitching to 201\n"
    );
}

#[test]
fn init_does_nothing_when_already_initialised() {
    let wm = FakeWm::new(&[("DP-2", 1920, "201"), ("HDMI-1", 0, "101")], "201");
    let run = run(wm, &["--init"]);
    assert!(run.output.status.success(), "{}", run.stderr());
    assert_eq!(run.commands(), [] as [&str; 0]);
    assert_eq!(run.stdout(), "101 HDMI-1\n201 DP-2\n");
}

#[test]
fn reports_rejected_commands_but_keeps_going() {
    let mut wm = two_outputs("101");
    wm.reject_moves = true;
    let run = run(wm, &["--shift", "next"]);
    assert_eq!(run.output.status.code(), Some(1));
    assert_eq!(
        run.commands(),
        ["move window to workspace 205", "workspace 205"]
    );
    assert!(
        run.stderr().contains("Nothing to move: workspace empty"),
        "{}",
        run.stderr()
    );
}

#[test]
fn fails_without_a_focused_workspace() {
    let run = run(two_outputs("none"), &["next"]);
    assert_eq!(run.output.status.code(), Some(1));
    assert_eq!(run.stderr(), "i3-ws: no focused workspace found\n");
    assert_eq!(run.types(), [GET_OUTPUTS, GET_WORKSPACES]);
}

#[test]
fn prints_usage_for_invalid_arguments() {
    let path = socket_path();
    for args in [&[][..], &["11"], &["--init", "1"], &["next", "--bogus"]] {
        let output = i3_ws(&path, args);
        assert_eq!(output.status.code(), Some(255), "{args:?}");
        assert_eq!(
            std::str::from_utf8(&output.stdout).unwrap(),
            "Usage: i3-ws [--init]\n\
             Usage: i3-ws [--ws] [--shift] {NUMBER}\n\
             Usage: i3-ws [--ws] [--shift] [--loop] [--create] (prev|next)\n"
        );
    }
}

#[test]
fn fails_when_the_socket_is_missing() {
    let path = socket_path();
    let output = i3_ws(&path, &["next"]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = std::str::from_utf8(&output.stderr).unwrap();
    assert!(stderr.starts_with("i3-ws: cannot connect to "), "{stderr}");
}
