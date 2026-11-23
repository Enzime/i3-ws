# i3-ws

Per-monitor workspaces for [i3](https://i3wm.org) and [sway](https://swaywm.org).

## Workspace numbering

Active outputs are numbered from 1, left to right. They are sorted by x
position, then width, then y position, then height. Disabled outputs (and
sway's non-desktop outputs) are skipped.

Output `N` gets the workspaces `N01` to `N10`. Workspace `k` of output `N` is
named `N*100+k`: on the second output from the left, workspace 3 is `203`.

## Usage

```
i3-ws [--init]
i3-ws [--ws] [--shift] {NUMBER}
i3-ws [--ws] [--shift] [--loop] [--create] (prev|next)
```

- `i3-ws NUMBER`, `i3-ws prev`, `i3-ws next`: focus output `NUMBER`, or the
  output to the left or right, showing the workspace that's already visible
  there. Does nothing with only one output.
- `--ws`: work on the workspaces of the focused output instead. `i3-ws --ws 3`
  switches to its workspace 3. `prev` and `next` step through the workspaces
  that exist on the output. With `--create` they step through all ten
  instead, creating workspaces as needed.
- `--shift`: move the focused window instead of switching. When moving to
  another output, focus follows the window. When moving to another workspace
  with `--ws`, it doesn't.
- `--loop`: wrap around at the first and last output or workspace.
- `--init`: show workspace `N01` on every output `N`. Run this once at
  startup.

Example i3 or sway config:

```
exec i3-ws --init

bindsym $mod+1 exec i3-ws --ws 1
bindsym $mod+Shift+1 exec i3-ws --ws --shift 1
bindsym $mod+Tab exec i3-ws --ws --loop next
bindsym $mod+period exec i3-ws next
bindsym $mod+Shift+period exec i3-ws --shift next
```

## Finding the window manager

The IPC socket is taken from `$SWAYSOCK`, then `$I3SOCK`, then
`i3 --get-socketpath`. sway sets both variables for everything it starts. i3
sets `$I3SOCK` for programs it starts with `exec`, and `i3 --get-socketpath`
finds it from anywhere else in the X session.
