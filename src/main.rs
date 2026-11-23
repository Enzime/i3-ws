//! Per-monitor workspaces for i3.

mod cli;
mod ipc;
mod plan;

use std::process::ExitCode;

use crate::ipc::Connection;
use crate::plan::{Action, Plan};

fn main() -> ExitCode {
    let args = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned());
    let Some(args) = cli::parse(args) else {
        print!("{}", cli::USAGE);
        // The C++ version returned -1 from main.
        return ExitCode::from(255);
    };

    match run(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("i3-ws: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Returns whether every command succeeded.
fn run(args: &cli::Args) -> Result<bool, String> {
    let mut connection = Connection::connect(&ipc::socket_path()?)?;
    let outputs = connection.get_outputs()?;
    let workspaces = connection.get_workspaces()?;

    match plan::plan(args, &outputs, &workspaces)? {
        Plan::Actions(actions) => perform(&mut connection, &actions),
        Plan::Init(targets) => {
            let mut ok = true;
            for target in &targets {
                let workspaces = connection.get_workspaces()?;
                let focused = plan::focused_workspace(&workspaces)?;
                ok &= perform(&mut connection, &plan::init_step(target, &focused.name))?;
            }
            Ok(ok)
        }
    }
}

/// Performs the actions in order. A command the window manager rejects is
/// reported but doesn't stop the remaining actions, like the C++ version.
/// Returns whether every command succeeded.
fn perform(connection: &mut Connection, actions: &[Action]) -> Result<bool, String> {
    let mut ok = true;
    for action in actions {
        match action {
            Action::Print(line) => println!("{line}"),
            Action::Command(command) => {
                for outcome in connection.run_command(command)? {
                    if !outcome.success {
                        ok = false;
                        eprintln!(
                            "i3-ws: command {command:?} failed: {}",
                            outcome.error.as_deref().unwrap_or("unknown error")
                        );
                    }
                }
            }
        }
    }
    Ok(ok)
}
