//! The decision logic: given the parsed arguments and the window manager's
//! outputs and workspaces, decide what to print and which commands to run.
//!
//! Nothing in here talks to the window manager, so every mode can be tested
//! with plain data.

use std::cmp::Ordering;

use serde::Deserialize;

use crate::cli::{Args, MAX_WORKSPACES_PER_OUTPUT, Mode, Target, stream_to_int};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Rect {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

/// An entry of the GET_OUTPUTS reply. Missing keys get their defaults, so
/// entries without `active` count as inactive.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Output {
    pub name: String,
    pub active: bool,
    pub current_workspace: Option<String>,
    pub rect: Rect,
}

/// An entry of the GET_WORKSPACES reply.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub name: String,
    pub focused: bool,
    pub output: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// A line of informational output on stdout.
    Print(String),
    /// A RUN_COMMAND payload.
    Command(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Perform these actions in order. Empty means there is nothing to do.
    Actions(Vec<Action>),
    /// `--init`: for each target in order, re-read the focused workspace and
    /// perform [`init_step`].
    Init(Vec<InitTarget>),
}

/// `--init` work for one output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitTarget {
    pub output: String,
    /// The output's visible workspace when the outputs were read.
    pub current_workspace: String,
    /// The workspace that should end up visible on the output (`N*100+1`).
    pub workspace: String,
}

/// Orders outputs left to right: by x, then width, then y, then height.
pub fn compare_rect(a: &Rect, b: &Rect) -> Ordering {
    (a.x, a.width, a.y, a.height).cmp(&(b.x, b.width, b.y, b.height))
}

/// The active outputs, sorted with [`compare_rect`]. Output `N` is at index
/// `N - 1`. Outputs with identical rects keep their IPC order.
pub fn active_outputs(outputs: &[Output]) -> Vec<&Output> {
    let mut active: Vec<&Output> = outputs.iter().filter(|o| o.active).collect();
    active.sort_by(|a, b| compare_rect(&a.rect, &b.rect));
    active
}

pub fn focused_workspace(workspaces: &[Workspace]) -> Result<&Workspace, String> {
    workspaces
        .iter()
        .find(|w| w.focused)
        .ok_or_else(|| "no focused workspace found".to_owned())
}

/// The name of workspace `index` (1-based) on the output at `output_index`.
fn workspace_name(output_index: usize, index: i64) -> String {
    ((output_index as i64 + 1) * 100 + index).to_string()
}

pub fn plan(args: &Args, outputs: &[Output], workspaces: &[Workspace]) -> Result<Plan, String> {
    let active = active_outputs(outputs);
    let focused = focused_workspace(workspaces)?;
    let output_index = active
        .iter()
        .position(|o| Some(&o.name) == focused.output.as_ref())
        .ok_or_else(|| {
            format!(
                "the focused workspace {:?} is not on an active output",
                focused.name
            )
        })?;

    let &Args::Switch {
        mode,
        target,
        shift,
        wrap,
        create,
    } = args
    else {
        return active
            .iter()
            .enumerate()
            .map(|(i, o)| {
                Ok(InitTarget {
                    output: o.name.clone(),
                    current_workspace: current_workspace(o)?.to_owned(),
                    workspace: workspace_name(i, 1),
                })
            })
            .collect::<Result<_, _>>()
            .map(Plan::Init);
    };

    let command = if shift {
        "move window to workspace "
    } else {
        "workspace "
    };

    let actions = match mode {
        Mode::Output if active.len() > 1 => {
            switch_output(&active, output_index, target, shift, wrap, command)?
        }
        Mode::Output => Vec::new(),
        Mode::Workspace => switch_workspace(
            workspaces,
            focused,
            output_index,
            target,
            wrap,
            create,
            command,
        )?,
    };

    Ok(Plan::Actions(actions))
}

fn switch_output(
    active: &[&Output],
    output_index: usize,
    target: Target,
    shift: bool,
    wrap: bool,
    command: &str,
) -> Result<Vec<Action>, String> {
    let len = active.len() as i64;
    let current = output_index as i64;

    let mut index = match target {
        Target::Number(n) if i64::from(n) - 1 != current => i64::from(n) - 1,
        Target::Prev => current - 1,
        Target::Next => current + 1,
        Target::Number(_) => return Ok(Vec::new()),
    };

    if wrap {
        index = index.rem_euclid(len);
    } else if !(0..len).contains(&index) {
        return Ok(Vec::new());
    }

    let output = active[index as usize];
    let workspace = current_workspace(output)?;

    let mut actions = vec![
        Action::Print(index.to_string()),
        Action::Print(format!("{} {}", output.name, workspace)),
        Action::Command(format!("{command}{workspace}")),
    ];
    // Moving a window to another output puts it on a visible workspace, so
    // follow it there.
    if shift {
        actions.push(Action::Command(format!("workspace {workspace}")));
    }
    Ok(actions)
}

/// The workspace visible on an active output. i3 and sway only report `null`
/// for inactive outputs, so this shouldn't fail; the C++ version would have
/// sent a command with an empty workspace name instead.
fn current_workspace(output: &Output) -> Result<&str, String> {
    output
        .current_workspace
        .as_deref()
        .ok_or_else(|| format!("output {:?} has no current workspace", output.name))
}

fn switch_workspace(
    workspaces: &[Workspace],
    focused: &Workspace,
    output_index: usize,
    target: Target,
    wrap: bool,
    create: bool,
    command: &str,
) -> Result<Vec<Action>, String> {
    let mut actions = vec![Action::Print(focused.name.clone())];

    // The candidate workspace indices and the position of the focused
    // workspace among them. Positions are signed because the C++ computed
    // them with unchecked iterator arithmetic; see the range check below.
    let (indices, position): (Vec<i64>, i64) = match target {
        Target::Number(n) => {
            let n = i64::from(n);
            (vec![n], 0)
        }
        _ if create => (
            (1..=i64::from(MAX_WORKSPACES_PER_OUTPUT)).collect(),
            i64::from(stream_to_int(&focused.name) % 100) - 1,
        ),
        _ => {
            let on_output: Vec<&Workspace> = workspaces
                .iter()
                .filter(|w| w.output == focused.output)
                .collect();
            let position = on_output
                .iter()
                .position(|w| std::ptr::eq(*w, focused))
                .expect("the focused workspace is on the focused output");
            (
                on_output
                    .iter()
                    .map(|w| i64::from(stream_to_int(&w.name) % 100))
                    .collect(),
                position as i64,
            )
        }
    };

    let last = indices.len() as i64 - 1;
    let new_position = match target {
        Target::Number(_) => position,
        Target::Prev if position != 0 => position - 1,
        Target::Prev if wrap => last,
        Target::Prev => {
            actions.push(Action::Print("At the first workspace, use --loop".into()));
            return Ok(actions);
        }
        Target::Next if position != last => position + 1,
        Target::Next if wrap => 0,
        Target::Next => {
            actions.push(Action::Print("At the last workspace, use --loop".into()));
            return Ok(actions);
        }
    };

    let Some(&index) = usize::try_from(new_position)
        .ok()
        .and_then(|p| indices.get(p))
    else {
        return Err(format!(
            "the focused workspace {:?} is not one of this tool's workspaces \
             (N01 to N{MAX_WORKSPACES_PER_OUTPUT:02}), so --create cannot step from it",
            focused.name
        ));
    };

    let name = workspace_name(output_index, index);
    if name != focused.name {
        actions.push(Action::Command(format!("{command}{name}")));
    }
    Ok(actions)
}

/// The `--init` actions for one output, given the name of the workspace that
/// is focused right now.
pub fn init_step(target: &InitTarget, focused: &str) -> Vec<Action> {
    let mut actions = vec![Action::Print(format!(
        "{} {}",
        target.workspace, target.output
    ))];

    if target.current_workspace != target.workspace {
        // Focus the output first by switching to its visible workspace, so
        // the new workspace is created there.
        if target.current_workspace != focused {
            actions.extend([
                Action::Print(format!("Switching from {focused}")),
                Action::Print(format!("Switching to {}", target.current_workspace)),
                Action::Command(format!("workspace {}", target.current_workspace)),
            ]);
        }
        actions.extend([
            Action::Print(format!("Switching to {}", target.workspace)),
            Action::Command(format!("workspace {}", target.workspace)),
        ]);
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli;

    fn output(name: &str, (x, width, y, height): (i64, i64, i64, i64), current: &str) -> Output {
        Output {
            name: name.into(),
            active: true,
            current_workspace: Some(current.into()),
            rect: Rect {
                x,
                y,
                width,
                height,
            },
        }
    }

    fn inactive(name: &str) -> Output {
        Output {
            name: name.into(),
            ..Output::default()
        }
    }

    fn workspace(name: &str, output: &str) -> Workspace {
        Workspace {
            name: name.into(),
            focused: false,
            output: Some(output.into()),
        }
    }

    /// Three outputs, deliberately not in left-to-right order, plus i3's
    /// inactive `xroot-0` pseudo-output.
    fn outputs() -> Vec<Output> {
        vec![
            inactive("xroot-0"),
            output("DP-2", (1920, 2560, 0, 1440), "205"),
            output("DP-1", (4480, 1920, 0, 1080), "301"),
            output("HDMI-1", (0, 1920, 0, 1080), "101"),
        ]
    }

    fn workspaces(focused: &str) -> Vec<Workspace> {
        let mut workspaces = vec![
            workspace("101", "HDMI-1"),
            workspace("103", "HDMI-1"),
            workspace("202", "DP-2"),
            workspace("205", "DP-2"),
            workspace("207", "DP-2"),
            workspace("301", "DP-1"),
        ];
        for w in &mut workspaces {
            w.focused = w.name == focused;
        }
        workspaces
    }

    fn plan_for(args: &[&str], outputs: &[Output], workspaces: &[Workspace]) -> Plan {
        let args = cli::parse(args).expect("valid arguments");
        plan(&args, outputs, workspaces).expect("plan succeeds")
    }

    fn commands(plan: &Plan) -> Vec<&str> {
        let Plan::Actions(actions) = plan else {
            panic!("expected actions, got {plan:?}");
        };
        actions
            .iter()
            .filter_map(|a| match a {
                Action::Command(c) => Some(c.as_str()),
                Action::Print(_) => None,
            })
            .collect()
    }

    /// The commands for `args` with the standard outputs and `focused`.
    fn run(args: &[&str], focused: &str) -> Vec<String> {
        let plan = plan_for(args, &outputs(), &workspaces(focused));
        commands(&plan).into_iter().map(str::to_owned).collect()
    }

    fn rect(x: i64, width: i64, y: i64, height: i64) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn compare_rect_orders_by_x_width_y_height() {
        use Ordering::*;
        assert_eq!(compare_rect(&rect(0, 9, 9, 9), &rect(1, 0, 0, 0)), Less);
        assert_eq!(compare_rect(&rect(1, 0, 0, 0), &rect(0, 9, 9, 9)), Greater);
        assert_eq!(compare_rect(&rect(5, 1, 9, 9), &rect(5, 2, 0, 0)), Less);
        assert_eq!(compare_rect(&rect(5, 2, 0, 0), &rect(5, 1, 9, 9)), Greater);
        assert_eq!(compare_rect(&rect(5, 2, 1, 9), &rect(5, 2, 2, 0)), Less);
        assert_eq!(compare_rect(&rect(5, 2, 2, 0), &rect(5, 2, 1, 9)), Greater);
        assert_eq!(compare_rect(&rect(5, 2, 2, 1), &rect(5, 2, 2, 3)), Less);
        assert_eq!(compare_rect(&rect(5, 2, 2, 3), &rect(5, 2, 2, 1)), Greater);
        assert_eq!(compare_rect(&rect(5, 2, 2, 3), &rect(5, 2, 2, 3)), Equal);
    }

    #[test]
    fn active_outputs_are_filtered_and_sorted() {
        let outputs = vec![
            output("wide", (0, 2560, 0, 1440), "1"),
            inactive("off"),
            output("right", (1920, 1920, 0, 1080), "2"),
            output("tall", (0, 1920, 0, 1200), "3"),
            output("below", (0, 1920, 1080, 1080), "4"),
            output("short", (0, 1920, 0, 1080), "5"),
            output("same-as-short", (0, 1920, 0, 1080), "6"),
        ];
        let names: Vec<&str> = active_outputs(&outputs)
            .iter()
            .map(|o| o.name.as_str())
            .collect();
        // Same x: narrower first; same width: higher first; same y: shorter
        // first; identical rects keep their IPC order.
        assert_eq!(
            names,
            ["short", "same-as-short", "tall", "below", "wide", "right"]
        );
    }

    #[test]
    fn output_number() {
        assert_eq!(run(&["1"], "205"), ["workspace 101"]);
        assert_eq!(run(&["3"], "205"), ["workspace 301"]);
        // Already on that output.
        assert_eq!(run(&["2"], "205"), [] as [&str; 0]);
        // No such output.
        assert_eq!(run(&["4"], "205"), [] as [&str; 0]);
    }

    #[test]
    fn output_number_prints_index_and_target() {
        let plan = plan_for(&["3"], &outputs(), &workspaces("205"));
        assert_eq!(
            plan,
            Plan::Actions(vec![
                Action::Print("2".into()),
                Action::Print("DP-1 301".into()),
                Action::Command("workspace 301".into()),
            ])
        );
    }

    #[test]
    fn output_prev_next() {
        assert_eq!(run(&["next"], "205"), ["workspace 301"]);
        assert_eq!(run(&["prev"], "205"), ["workspace 101"]);
        assert_eq!(run(&["next"], "101"), ["workspace 205"]);
        assert_eq!(run(&["prev"], "301"), ["workspace 205"]);
        // At the edges without --loop.
        assert_eq!(run(&["next"], "301"), [] as [&str; 0]);
        assert_eq!(run(&["prev"], "103"), [] as [&str; 0]);
    }

    #[test]
    fn output_loop() {
        assert_eq!(run(&["--loop", "next"], "301"), ["workspace 101"]);
        assert_eq!(run(&["--loop", "prev"], "101"), ["workspace 301"]);
        assert_eq!(run(&["--loop", "next"], "205"), ["workspace 301"]);
        // --loop also wraps output numbers past the last output, like the
        // C++. This can target the focused output itself.
        assert_eq!(run(&["--loop", "5"], "101"), ["workspace 205"]);
        assert_eq!(run(&["--loop", "4"], "101"), ["workspace 101"]);
    }

    #[test]
    fn output_shift_moves_then_follows() {
        assert_eq!(
            run(&["--shift", "3"], "101"),
            ["move window to workspace 301", "workspace 301"]
        );
        assert_eq!(
            run(&["--shift", "--loop", "prev"], "101"),
            ["move window to workspace 301", "workspace 301"]
        );
        assert_eq!(run(&["--shift", "1"], "101"), [] as [&str; 0]);
    }

    #[test]
    fn output_mode_is_a_no_op_with_one_active_output() {
        let outputs = vec![
            inactive("xroot-0"),
            output("eDP-1", (0, 1920, 0, 1080), "102"),
            inactive("HDMI-1"),
        ];
        let workspaces = vec![
            workspace("101", "eDP-1"),
            Workspace {
                focused: true,
                ..workspace("102", "eDP-1")
            },
        ];
        for args in [
            &["next"][..],
            &["prev"],
            &["--loop", "next"],
            &["--loop", "2"],
            &["1"],
            &["--shift", "2"],
        ] {
            let plan = plan_for(args, &outputs, &workspaces);
            assert_eq!(plan, Plan::Actions(Vec::new()), "{args:?}");
        }
        // --ws still works.
        let plan = plan_for(&["--ws", "4"], &outputs, &workspaces);
        assert_eq!(commands(&plan), ["workspace 104"]);
    }

    #[test]
    fn workspace_number() {
        assert_eq!(run(&["--ws", "3"], "205"), ["workspace 203"]);
        assert_eq!(run(&["--ws", "10"], "205"), ["workspace 210"]);
        assert_eq!(run(&["--ws", "4"], "301"), ["workspace 304"]);
        assert_eq!(run(&["--ws", "2"], "103"), ["workspace 102"]);
        // Already there.
        assert_eq!(run(&["--ws", "5"], "205"), [] as [&str; 0]);
    }

    #[test]
    fn workspace_shift_does_not_follow() {
        assert_eq!(
            run(&["--ws", "--shift", "9"], "205"),
            ["move window to workspace 209"]
        );
        assert_eq!(
            run(&["--ws", "--shift", "next"], "205"),
            ["move window to workspace 207"]
        );
        assert_eq!(run(&["--ws", "--shift", "5"], "205"), [] as [&str; 0]);
    }

    #[test]
    fn workspace_prev_next_walk_existing_workspaces() {
        assert_eq!(run(&["--ws", "next"], "205"), ["workspace 207"]);
        assert_eq!(run(&["--ws", "prev"], "205"), ["workspace 202"]);
        assert_eq!(run(&["--ws", "next"], "101"), ["workspace 103"]);
    }

    #[test]
    fn workspace_prev_next_stop_at_the_edges() {
        let plan = plan_for(&["--ws", "next"], &outputs(), &workspaces("207"));
        assert_eq!(
            plan,
            Plan::Actions(vec![
                Action::Print("207".into()),
                Action::Print("At the last workspace, use --loop".into()),
            ])
        );
        let plan = plan_for(&["--ws", "prev"], &outputs(), &workspaces("202"));
        assert_eq!(
            plan,
            Plan::Actions(vec![
                Action::Print("202".into()),
                Action::Print("At the first workspace, use --loop".into()),
            ])
        );
        // The only workspace on its output is both first and last.
        assert_eq!(run(&["--ws", "next"], "301"), [] as [&str; 0]);
        assert_eq!(run(&["--ws", "prev"], "301"), [] as [&str; 0]);
    }

    #[test]
    fn workspace_loop() {
        assert_eq!(run(&["--ws", "--loop", "next"], "207"), ["workspace 202"]);
        assert_eq!(run(&["--ws", "--loop", "prev"], "202"), ["workspace 207"]);
        // Wrapping back onto the focused workspace sends nothing.
        assert_eq!(run(&["--ws", "--loop", "next"], "301"), [] as [&str; 0]);
    }

    #[test]
    fn workspace_create_steps_through_all_ten() {
        assert_eq!(run(&["--ws", "--create", "next"], "205"), ["workspace 206"]);
        assert_eq!(run(&["--ws", "--create", "prev"], "205"), ["workspace 204"]);
        assert_eq!(run(&["--ws", "--create", "next"], "101"), ["workspace 102"]);
        assert_eq!(run(&["--ws", "--create", "prev"], "101"), [] as [&str; 0]);
        assert_eq!(
            run(&["--ws", "--create", "--loop", "prev"], "101"),
            ["workspace 110"]
        );
        assert_eq!(
            run(&["--ws", "--create", "--shift", "next"], "301"),
            ["move window to workspace 302"]
        );
    }

    #[test]
    fn workspace_create_at_the_last_workspace() {
        let outputs = vec![output("HDMI-1", (0, 1920, 0, 1080), "110")];
        let workspaces = vec![Workspace {
            focused: true,
            ..workspace("110", "HDMI-1")
        }];
        let plan = plan_for(&["--ws", "--create", "next"], &outputs, &workspaces);
        assert_eq!(commands(&plan), [] as [&str; 0]);
        let plan = plan_for(
            &["--ws", "--create", "--loop", "next"],
            &outputs,
            &workspaces,
        );
        assert_eq!(commands(&plan), ["workspace 101"]);
    }

    #[test]
    fn workspace_create_from_a_foreign_workspace() {
        let outputs = vec![output("HDMI-1", (0, 1920, 0, 1080), "mail")];
        let focused = |name: &str| {
            vec![Workspace {
                focused: true,
                ..workspace(name, "HDMI-1")
            }]
        };
        let args = cli::parse(["--ws", "--create", "prev"]).unwrap();
        // "mail" counts as index 0, one before the first workspace.
        assert!(plan(&args, &outputs, &focused("mail")).is_err());
        let args = cli::parse(["--ws", "--create", "next"]).unwrap();
        let plan_next = plan(&args, &outputs, &focused("mail")).unwrap();
        assert_eq!(commands(&plan_next), ["workspace 101"]);
        // "111" counts as index 11, one past the last workspace.
        assert!(plan(&args, &outputs, &focused("111")).is_err());
        let args = cli::parse(["--ws", "--create", "prev"]).unwrap();
        let plan_prev = plan(&args, &outputs, &focused("111")).unwrap();
        assert_eq!(commands(&plan_prev), ["workspace 110"]);
    }

    #[test]
    fn workspace_names_are_parsed_like_stringstream() {
        let outputs = vec![output("HDMI-1", (0, 1920, 0, 1080), "3: mail")];
        let workspaces = vec![
            workspace("101", "HDMI-1"),
            Workspace {
                focused: true,
                ..workspace("3: mail", "HDMI-1")
            },
            workspace("web", "HDMI-1"),
        ];
        let plan_prev = plan_for(&["--ws", "prev"], &outputs, &workspaces);
        assert_eq!(commands(&plan_prev), ["workspace 101"]);
        // "web" has no leading number, so it counts as index 0.
        let plan_next = plan_for(&["--ws", "next"], &outputs, &workspaces);
        assert_eq!(commands(&plan_next), ["workspace 100"]);
    }

    #[test]
    fn init_targets_every_active_output_left_to_right() {
        let plan = plan_for(&["--init"], &outputs(), &workspaces("205"));
        let target = |output: &str, current: &str, workspace: &str| InitTarget {
            output: output.into(),
            current_workspace: current.into(),
            workspace: workspace.into(),
        };
        assert_eq!(
            plan,
            Plan::Init(vec![
                target("HDMI-1", "101", "101"),
                target("DP-2", "205", "201"),
                target("DP-1", "301", "301"),
            ])
        );
    }

    #[test]
    fn init_step() {
        let target = InitTarget {
            output: "DP-2".into(),
            current_workspace: "205".into(),
            workspace: "201".into(),
        };
        // The output is focused: just switch.
        assert_eq!(
            super::init_step(&target, "205"),
            [
                Action::Print("201 DP-2".into()),
                Action::Print("Switching to 201".into()),
                Action::Command("workspace 201".into()),
            ]
        );
        // Another output is focused: focus this one first.
        assert_eq!(
            super::init_step(&target, "101"),
            [
                Action::Print("201 DP-2".into()),
                Action::Print("Switching from 101".into()),
                Action::Print("Switching to 205".into()),
                Action::Command("workspace 205".into()),
                Action::Print("Switching to 201".into()),
                Action::Command("workspace 201".into()),
            ]
        );
        // Already showing its first workspace.
        let done = InitTarget {
            current_workspace: "201".into(),
            ..target
        };
        assert_eq!(
            super::init_step(&done, "101"),
            [Action::Print("201 DP-2".into())]
        );
    }

    #[test]
    fn missing_focus_is_an_error() {
        for args in [&["--init"][..], &["next"], &["--ws", "1"]] {
            let args = cli::parse(args).unwrap();
            let error = plan(&args, &outputs(), &workspaces("none")).unwrap_err();
            assert_eq!(error, "no focused workspace found");
        }
    }

    #[test]
    fn focus_on_an_inactive_output_is_an_error() {
        let mut workspaces = workspaces("none");
        workspaces.push(Workspace {
            focused: true,
            ..workspace("401", "xroot-0")
        });
        let args = cli::parse(["--ws", "next"]).unwrap();
        let error = plan(&args, &outputs(), &workspaces).unwrap_err();
        assert!(error.contains("not on an active output"), "{error}");
    }
}

#[cfg(test)]
mod sway_tests {
    use super::*;
    use crate::cli;

    /// A GET_OUTPUTS reply in sway's format: an enabled output left of the
    /// origin, a disabled output and a non-desktop output (which has no
    /// `active`, `rect` or `current_workspace` keys).
    const OUTPUTS: &str = r#"[
        {
            "id": 4, "type": "output", "name": "DP-1", "make": "Dell",
            "model": "U2720Q", "serial": "ABC", "active": true, "dpms": true,
            "power": true, "primary": false, "scale": 1.5,
            "transform": "normal", "current_workspace": "205",
            "modes": [], "current_mode": {"width": 3840, "height": 2160, "refresh": 60000},
            "rect": {"x": 0, "y": 0, "width": 2560, "height": 1440},
            "focused": true, "non_desktop": false
        },
        {
            "id": 5, "type": "output", "name": "eDP-1", "active": true,
            "primary": false, "current_workspace": "101",
            "rect": {"x": -1920, "y": 360, "width": 1920, "height": 1200},
            "focused": false, "non_desktop": false
        },
        {
            "type": "output", "name": "HDMI-A-1", "active": false,
            "dpms": false, "power": false, "primary": false, "percent": null,
            "current_workspace": null,
            "rect": {"x": 0, "y": 0, "width": 0, "height": 0},
            "non_desktop": false
        },
        {
            "type": "output", "name": "HMD-1", "make": "Valve",
            "model": "Index", "serial": "", "non_desktop": true,
            "modes": [], "features": {}, "primary": false
        }
    ]"#;

    fn workspace_json(name: &str, output: &str, focused: bool) -> String {
        format!(
            r#"{{
                "id": 10, "type": "workspace", "num": {name}, "name": "{name}",
                "output": "{output}", "focused": {focused}, "visible": true,
                "urgent": false, "representation": "H[foot]",
                "layout": "splith", "orientation": "horizontal",
                "rect": {{"x": 0, "y": 0, "width": 1920, "height": 1200}},
                "floating_nodes": [], "nodes": [], "focus": []
            }}"#
        )
    }

    fn workspaces(focused: &str) -> Vec<Workspace> {
        let entries: Vec<String> = [("101", "eDP-1"), ("205", "DP-1"), ("207", "DP-1")]
            .iter()
            .map(|&(name, output)| workspace_json(name, output, name == focused))
            .collect();
        serde_json::from_str(&format!("[{}]", entries.join(","))).unwrap()
    }

    fn outputs() -> Vec<Output> {
        serde_json::from_str(OUTPUTS).unwrap()
    }

    fn commands(args: &[&str], outputs: &[Output], focused: &str) -> Result<Vec<String>, String> {
        let args = cli::parse(args).unwrap();
        match plan(&args, outputs, &workspaces(focused))? {
            Plan::Actions(actions) => Ok(actions
                .into_iter()
                .filter_map(|a| match a {
                    Action::Command(c) => Some(c),
                    Action::Print(_) => None,
                })
                .collect()),
            Plan::Init(targets) => Ok(targets
                .iter()
                .map(|t| format!("{} {} {}", t.output, t.current_workspace, t.workspace))
                .collect()),
        }
    }

    #[test]
    fn parses_sway_outputs() {
        let outputs = outputs();
        assert_eq!(outputs.len(), 4);
        assert_eq!(outputs[2].current_workspace, None);
        assert!(!outputs[3].active);
        assert_eq!(outputs[3].current_workspace, None);
        assert_eq!(outputs[3].rect, Rect::default());
    }

    #[test]
    fn only_enabled_outputs_are_numbered_and_negative_positions_sort_first() {
        let outputs = outputs();
        let names: Vec<&str> = active_outputs(&outputs)
            .iter()
            .map(|o| o.name.as_str())
            .collect();
        assert_eq!(names, ["eDP-1", "DP-1"]);
    }

    #[test]
    fn switches_outputs() {
        let outputs = outputs();
        assert_eq!(
            commands(&["prev"], &outputs, "205").unwrap(),
            ["workspace 101"]
        );
        assert_eq!(
            commands(&["next"], &outputs, "205").unwrap(),
            [] as [&str; 0]
        );
        assert_eq!(
            commands(&["2"], &outputs, "101").unwrap(),
            ["workspace 205"]
        );
        // There is no third output: the disabled and non-desktop ones don't count.
        assert_eq!(commands(&["3"], &outputs, "101").unwrap(), [] as [&str; 0]);
    }

    #[test]
    fn switches_workspaces() {
        let outputs = outputs();
        assert_eq!(
            commands(&["--ws", "3"], &outputs, "205").unwrap(),
            ["workspace 203"]
        );
        assert_eq!(
            commands(&["--ws", "next"], &outputs, "205").unwrap(),
            ["workspace 207"]
        );
        assert_eq!(
            commands(&["--ws", "--shift", "2"], &outputs, "101").unwrap(),
            ["move window to workspace 102"]
        );
    }

    #[test]
    fn init() {
        assert_eq!(
            commands(&["--init"], &outputs(), "205").unwrap(),
            ["eDP-1 101 101", "DP-1 205 201"]
        );
    }

    #[test]
    fn active_output_without_a_current_workspace() {
        let mut outputs = outputs();
        outputs[1].current_workspace = None;
        // Targeting it is an error instead of sending `workspace ` with no name.
        let error = commands(&["prev"], &outputs, "205").unwrap_err();
        assert_eq!(error, r#"output "eDP-1" has no current workspace"#);
        let error = commands(&["--init"], &outputs, "205").unwrap_err();
        assert_eq!(error, r#"output "eDP-1" has no current workspace"#);
        // Anything that doesn't need it still works.
        assert_eq!(
            commands(&["--ws", "next"], &outputs, "205").unwrap(),
            ["workspace 207"]
        );
        assert_eq!(
            commands(&["next"], &outputs, "101").unwrap(),
            ["workspace 205"]
        );
    }
}
