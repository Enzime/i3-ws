//! Command-line parsing.
//!
//! This mirrors the argument loop of the original C++ implementation, quirks
//! included: flags may appear anywhere and repeat, a positional argument is
//! only accepted while no target has been set yet, and `0` (or an empty
//! string) counts as "no target" so a later positional may still follow.

/// Highest workspace index `k` in a workspace name `N*100+k`.
pub const MAX_WORKSPACES_PER_OUTPUT: i32 = 10;

pub const USAGE: &str = "\
Usage: i3-ws [--init]
Usage: i3-ws [--ws] [--shift] {NUMBER}
Usage: i3-ws [--ws] [--shift] [--loop] [--create] (prev|next)
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// 1-based output number (output mode) or workspace index (`--ws`).
    Number(i32),
    Prev,
    Next,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Switch between outputs (no `--ws`).
    Output,
    /// Switch between the workspaces of the focused output (`--ws`).
    Workspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Args {
    Init,
    Switch {
        mode: Mode,
        target: Target,
        shift: bool,
        wrap: bool,
        create: bool,
    },
}

/// Parses the arguments (without the program name). `None` means the usage
/// text should be printed.
pub fn parse<I, S>(args: I) -> Option<Args>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    // Same encoding as the C++: -2 = next, -1 = prev, 0 = unset, 1.. = number.
    let mut pos = 0;
    let mut mode = Mode::Output;
    let mut wrap = false;
    let mut create = false;
    let mut init = false;
    let mut shift = false;

    for arg in args {
        match arg.as_ref() {
            "prev" if pos == 0 => pos = -1,
            "next" if pos == 0 => pos = -2,
            a if pos == 0 && a.bytes().all(|b| b.is_ascii_digit()) => pos = stream_to_int(a),
            "--ws" => mode = Mode::Workspace,
            "--loop" => wrap = true,
            "--create" => create = true,
            "--init" => init = true,
            "--shift" => shift = true,
            _ => return None,
        }
    }

    if init {
        return (pos == 0).then_some(Args::Init);
    }

    let target = match pos {
        -2 => Target::Next,
        -1 => Target::Prev,
        1..=MAX_WORKSPACES_PER_OUTPUT => Target::Number(pos),
        _ => return None,
    };

    Some(Args::Switch {
        mode,
        target,
        shift,
        wrap,
        create,
    })
}

/// Converts the leading integer of `s` like `std::stringstream >> int`:
/// leading whitespace and a sign are accepted, parsing stops at the first
/// non-digit, no digits yield 0 and out-of-range values saturate.
pub fn stream_to_int(s: &str) -> i32 {
    let s = s.trim_start_matches([' ', '\t', '\n', '\x0b', '\x0c', '\r']);
    let (negative, rest) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };

    let mut value: i64 = 0;
    for digit in rest.bytes().take_while(u8::is_ascii_digit) {
        value = value
            .saturating_mul(10)
            .saturating_add(i64::from(digit - b'0'));
    }
    if negative {
        value = -value;
    }

    value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn switch(mode: Mode, target: Target) -> Args {
        Args::Switch {
            mode,
            target,
            shift: false,
            wrap: false,
            create: false,
        }
    }

    #[test]
    fn parses_targets() {
        assert_eq!(parse(["1"]), Some(switch(Mode::Output, Target::Number(1))));
        assert_eq!(
            parse(["10"]),
            Some(switch(Mode::Output, Target::Number(10)))
        );
        assert_eq!(
            parse(["007"]),
            Some(switch(Mode::Output, Target::Number(7)))
        );
        assert_eq!(parse(["prev"]), Some(switch(Mode::Output, Target::Prev)));
        assert_eq!(parse(["next"]), Some(switch(Mode::Output, Target::Next)));
    }

    #[test]
    fn parses_flags_in_any_position() {
        assert_eq!(
            parse(["next", "--create", "--ws", "--loop", "--shift", "--ws"]),
            Some(Args::Switch {
                mode: Mode::Workspace,
                target: Target::Next,
                shift: true,
                wrap: true,
                create: true,
            })
        );
    }

    #[test]
    fn parses_init() {
        assert_eq!(parse(["--init"]), Some(Args::Init));
        // Other flags are accepted and ignored with --init.
        assert_eq!(parse(["--ws", "--init", "--loop"]), Some(Args::Init));
        assert_eq!(parse(["--init", "0"]), Some(Args::Init));
    }

    #[test]
    fn zero_or_empty_does_not_set_a_target() {
        assert_eq!(
            parse(["0", "3"]),
            Some(switch(Mode::Output, Target::Number(3)))
        );
        assert_eq!(
            parse(["", "next"]),
            Some(switch(Mode::Output, Target::Next))
        );
        assert_eq!(parse(["0"]), None);
        assert_eq!(parse([""]), None);
    }

    #[test]
    fn rejects_invalid_arguments() {
        let empty: [&str; 0] = [];
        assert_eq!(parse(empty), None);
        assert_eq!(parse(["--ws"]), None);
        assert_eq!(parse(["11"]), None);
        assert_eq!(parse(["99999999999999999999"]), None);
        assert_eq!(parse(["-1"]), None);
        assert_eq!(parse(["+1"]), None);
        assert_eq!(parse(["1", "2"]), None);
        assert_eq!(parse(["next", "prev"]), None);
        assert_eq!(parse(["prev", "1"]), None);
        assert_eq!(parse(["--init", "1"]), None);
        assert_eq!(parse(["--init", "next"]), None);
        assert_eq!(parse(["--help"]), None);
        assert_eq!(parse(["up"]), None);
    }

    #[test]
    fn stream_to_int_matches_stringstream() {
        assert_eq!(stream_to_int("101"), 101);
        assert_eq!(stream_to_int("  42abc"), 42);
        assert_eq!(stream_to_int("3: www"), 3);
        assert_eq!(stream_to_int("+7"), 7);
        assert_eq!(stream_to_int("-15"), -15);
        assert_eq!(stream_to_int("www"), 0);
        assert_eq!(stream_to_int(""), 0);
        assert_eq!(stream_to_int("-"), 0);
        assert_eq!(stream_to_int("99999999999999999999"), i32::MAX);
        assert_eq!(stream_to_int("-99999999999999999999"), i32::MIN);
    }
}
