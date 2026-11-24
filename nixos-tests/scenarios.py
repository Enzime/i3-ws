# Helpers and scenarios shared by the VM tests. The test script defines these
# before including this file:
#
# - OUTPUTS: the names of the active outputs, in the order i3-ws numbers them
# - LAYOUT: the (x, y, width, height) of each active output
# - ENV: the environment variables i3-ws needs to find the window manager
# - WINDOW: a command that opens a window titled `{title}`
# - ipc(msg_type, payload): sends an IPC message and returns the parsed reply
# - wm_exec(command): runs a shell command with the window manager's `exec`

import json
import shlex


def i3_ws(*args: str, env: str | None = None, status: int = 0) -> tuple[str, str]:
    """Runs i3-ws as alice with only PATH and `env` (default: ENV) set.
    Checks its exit status and returns its stdout and stderr."""
    command = (
        "setpriv --reuid=alice --regid=users --init-groups env -i"
        f" PATH=/run/current-system/sw/bin {ENV if env is None else env}"
        f" i3-ws {shlex.join(args)} 2>/tmp/i3-ws.stderr"
    )
    actual, stdout = machine.execute(command)
    stderr = machine.succeed("cat /tmp/i3-ws.stderr")
    assert actual == status, (
        f"{command}: exit status {actual}, expected {status}\n"
        f"stdout:\n{stdout}\nstderr:\n{stderr}"
    )
    return stdout, stderr


def workspaces() -> list[str]:
    return [w["name"] for w in ipc("get_workspaces")]


def focused() -> str:
    return next(w["name"] for w in ipc("get_workspaces") if w["focused"])


def visible() -> list[str]:
    """The workspace visible on each output, in i3-ws order."""
    current = {o["name"]: o["current_workspace"] for o in ipc("get_outputs")}
    return [current[name] for name in OUTPUTS]


def is_window(node) -> bool:
    return node.get("window") is not None or node.get("app_id") is not None


def children(node) -> list:
    return node.get("nodes", []) + node.get("floating_nodes", [])


def windows() -> dict[str, str | None]:
    """Maps the title of every window to the name of its workspace."""
    found: dict[str, str | None] = {}

    def walk(node, workspace: str | None) -> None:
        if node["type"] == "workspace":
            workspace = node["name"]
        if is_window(node):
            found[node["name"]] = workspace
        for child in children(node):
            walk(child, workspace)

    walk(ipc("get_tree"), None)
    return found


def focused_window() -> str | None:
    def walk(node) -> str | None:
        if node["focused"] and is_window(node):
            return node["name"]
        for child in children(node):
            if (title := walk(child)) is not None:
                return title
        return None

    return walk(ipc("get_tree"))


def check(focus: str, shown: list[str] | None = None) -> None:
    """Checks the focused workspace and, if given, the visible workspaces."""
    assert focused() == focus, f"focused workspace: {focused()}, expected {focus}"
    if shown is not None:
        assert visible() == shown, f"visible workspaces: {visible()}, expected {shown}"


def check_windows(expected: dict[str, str], focus: str | None) -> None:
    """Checks the workspace of every window and the focused window."""
    assert windows() == expected, f"windows: {windows()}, expected {expected}"
    assert focused_window() == focus, (
        f"focused window: {focused_window()}, expected {focus}"
    )


def open_window(title: str) -> None:
    """Opens a window on the focused workspace and waits until it has focus."""
    workspace = focused()
    wm_exec(WINDOW.format(title=title))
    retry(lambda _: windows().get(title) == workspace and focused_window() == title)


def wait_for_outputs() -> None:
    """Waits until the window manager has set up the outputs in LAYOUT."""

    def ready(_: bool) -> bool:
        outputs = {
            o["name"]: tuple(o["rect"][k] for k in ("x", "y", "width", "height"))
            for o in ipc("get_outputs")
            if o.get("active") and o["current_workspace"] is not None
        }
        return outputs == LAYOUT

    retry(ready)


def run_scenarios() -> None:
    with subtest("--init shows workspace N01 on every output N"):
        stdout, _ = i3_ws("--init")
        for n, output in enumerate(OUTPUTS, 1):
            assert f"{n}01 {output}" in stdout.splitlines(), stdout
        check("401", ["101", "201", "301", "401"])
        assert sorted(workspaces()) == ["101", "201", "301", "401"], workspaces()
        # There is nothing left to do the second time.
        i3_ws("--init")
        check("401", ["101", "201", "301", "401"])

    with subtest("NUMBER focuses output NUMBER"):
        i3_ws("1")
        check("101")
        i3_ws("1")
        check("101")
        i3_ws("3")
        check("301")
        # There is no output 5.
        i3_ws("5")
        check("301", ["101", "201", "301", "401"])

    with subtest("prev and next focus the neighbouring output"):
        i3_ws("next")
        check("401")
        i3_ws("next")
        check("401")
        i3_ws("--loop", "next")
        check("101")
        i3_ws("prev")
        check("101")
        i3_ws("--loop", "prev")
        check("401")
        i3_ws("prev")
        check("301")
        i3_ws("prev")
        check("201", ["101", "201", "301", "401"])

    with subtest("--ws NUMBER switches workspaces on the focused output"):
        i3_ws("--ws", "3")
        check("203", ["101", "203", "301", "401"])
        assert "201" not in workspaces(), workspaces()
        i3_ws("--ws", "3")
        check("203")
        i3_ws("--ws", "1")
        check("201", ["101", "201", "301", "401"])

    with subtest("--ws --shift NUMBER moves the window but not the focus"):
        open_window("a")
        i3_ws("--ws", "--shift", "3")
        check("201", ["101", "201", "301", "401"])
        check_windows({"a": "203"}, None)

        # Output 2 ends up with windows on 201, 203 and 206.
        open_window("b")
        i3_ws("--ws", "6")
        open_window("c")
        check_windows({"a": "203", "b": "201", "c": "206"}, "c")

    with subtest("--ws prev and next step through the existing workspaces"):
        stdout, _ = i3_ws("--ws", "next")
        assert "At the last workspace, use --loop" in stdout, stdout
        check("206")
        i3_ws("--ws", "--loop", "next")
        check("201")
        stdout, _ = i3_ws("--ws", "prev")
        assert "At the first workspace, use --loop" in stdout, stdout
        check("201")
        i3_ws("--ws", "--loop", "prev")
        check("206")
        i3_ws("--ws", "prev")
        check("203")
        i3_ws("--ws", "prev")
        check("201")
        i3_ws("--ws", "next")
        check("203", ["101", "203", "301", "401"])

    with subtest("--ws --create prev and next step through all ten workspaces"):
        i3_ws("--ws", "--create", "next")
        check("204")
        i3_ws("--ws", "--create", "prev")
        check("203")
        assert "204" not in workspaces(), workspaces()
        i3_ws("--ws", "10")
        check("210")
        stdout, _ = i3_ws("--ws", "--create", "next")
        assert "At the last workspace, use --loop" in stdout, stdout
        check("210")
        i3_ws("--ws", "--create", "--loop", "next")
        check("201")
        stdout, _ = i3_ws("--ws", "--create", "prev")
        assert "At the first workspace, use --loop" in stdout, stdout
        check("201")
        i3_ws("--ws", "--create", "--loop", "prev")
        check("210")
        i3_ws("--ws", "--create", "prev")
        check("209", ["101", "209", "301", "401"])

    with subtest("--shift NUMBER moves the window to output NUMBER and follows it"):
        i3_ws("--ws", "3")
        check_windows({"a": "203", "b": "201", "c": "206"}, "a")
        i3_ws("--shift", "3")
        check("301", ["101", "203", "301", "401"])
        check_windows({"a": "301", "b": "201", "c": "206"}, "a")
        i3_ws("--shift", "prev")
        check("203", ["101", "203", "301", "401"])
        check_windows({"a": "203", "b": "201", "c": "206"}, "a")
        i3_ws("--shift", "4")
        check("401")
        check_windows({"a": "401", "b": "201", "c": "206"}, "a")
        # There is no output after 4.
        i3_ws("--shift", "next")
        check("401")
        check_windows({"a": "401", "b": "201", "c": "206"}, "a")
        i3_ws("--shift", "--loop", "next")
        check("101", ["101", "203", "301", "401"])
        check_windows({"a": "101", "b": "201", "c": "206"}, "a")

    with subtest("i3-ws run by the window manager"):
        wm_exec("i3-ws --ws 4")
        retry(lambda _: focused() == "104")
        check("104", ["104", "203", "301", "401"])

    with subtest("commands the window manager rejects"):
        # 104 is empty, so there is no window to move.
        _, stderr = i3_ws("--ws", "--shift", "2", status=1)
        assert 'i3-ws: command "move window to workspace 102" failed' in stderr, stderr
        check("104")
        assert "102" not in workspaces(), workspaces()
        # The focus still follows to output 2.
        _, stderr = i3_ws("--shift", "2", status=1)
        assert 'i3-ws: command "move window to workspace 203" failed' in stderr, stderr
        check("203", ["104", "203", "301", "401"])

    with subtest("workspaces outside N01 to N10"):
        error = "i3-ws: the focused workspace {} is not one of this tool's workspaces"
        ipc("command", "workspace mail")
        # "mail" counts as workspace 0, one before the first.
        _, stderr = i3_ws("--ws", "--create", "prev", status=1)
        assert error.format('"mail"') in stderr, stderr
        check("mail")
        i3_ws("--ws", "--create", "next")
        check("201")
        ipc("command", "workspace 211")
        # "211" counts as workspace 11, one past the last.
        _, stderr = i3_ws("--ws", "--create", "next", status=1)
        assert error.format('"211"') in stderr, stderr
        check("211")
        i3_ws("--ws", "--create", "prev")
        check("210", ["104", "210", "301", "401"])
