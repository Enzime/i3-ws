{ lib, ... }:

{
  name = "i3-ws-sway";

  nodes.machine =
    { i3-ws, pkgs, ... }:
    let
      # The same layout as the i3 test, plus a disabled output. sway lists
      # disabled outputs with a rect of all zeros, which would sort first.
      swayConfig = pkgs.writeText "sway-config" ''
        output HEADLESS-1 mode 1024x768 position 1024 0
        output HEADLESS-2 mode 1024x768 position 0 768
        output HEADLESS-3 mode 800x600 position 1024 768
        output HEADLESS-4 mode 1024x768 position 0 0
        output HEADLESS-5 disable
        focus_follows_mouse no
        mouse_warping none
        xwayland disable
      '';
    in
    {
      users.users.alice.isNormalUser = true;

      environment.systemPackages = [
        i3-ws
        pkgs.sway-unwrapped
      ];

      # For foot
      fonts.packages = [ pkgs.dejavu_fonts ];

      # The headless backend can have any number of outputs of any size and
      # doesn't need a GPU, input devices or a seat.
      systemd.services.sway = {
        wantedBy = [ "multi-user.target" ];
        environment = {
          WLR_BACKENDS = "headless";
          WLR_HEADLESS_OUTPUTS = "5";
          WLR_RENDERER = "pixman";
          XDG_RUNTIME_DIR = "/run/sway";
          SWAYSOCK = "/run/sway/ipc.sock";
        };
        # For the programs the test starts with sway's `exec`, which runs
        # them with `sh -c` from PATH.
        path = [
          i3-ws
          pkgs.bash
          pkgs.foot
        ];
        serviceConfig = {
          User = "alice";
          RuntimeDirectory = "sway";
          RuntimeDirectoryMode = "0700";
          # Unwrapped, because the wrapper starts a D-Bus session bus
          ExecStart = "${lib.getExe pkgs.sway-unwrapped} --config ${swayConfig}";
        };
      };
    };

  testScript = ''
    # i3-ws sorts outputs by x, then width, then y, then height.
    OUTPUTS = ["HEADLESS-4", "HEADLESS-2", "HEADLESS-3", "HEADLESS-1"]
    LAYOUT = {
        "HEADLESS-1": (1024, 0, 1024, 768),
        "HEADLESS-2": (0, 768, 1024, 768),
        "HEADLESS-3": (1024, 768, 800, 600),
        "HEADLESS-4": (0, 0, 1024, 768),
    }
    SOCKET = "/run/sway/ipc.sock"
    ENV = f"SWAYSOCK={SOCKET}"
    WINDOW = "foot --title={title} --app-id={title} sleep infinity"


    def ipc(msg_type: str, payload: str = ""):
        command = f"swaymsg -s {SOCKET} -r -t {msg_type}"
        if payload:
            command += f" -- {shlex.quote(payload)}"
        return json.loads(machine.succeed(command))


    def wm_exec(command: str) -> None:
        ipc("command", f"exec {command}")


    ${builtins.readFile ./scenarios.py}

    machine.wait_until_succeeds(f"swaymsg -s {SOCKET} -t get_version")
    wait_for_outputs()

    with subtest("the disabled output is listed"):
        (disabled,) = [o for o in ipc("get_outputs") if o["name"] == "HEADLESS-5"]
        assert disabled["active"] is False, disabled
        # Numbered, it would be output 1 and shift all the others.
        assert disabled["rect"] == {"x": 0, "y": 0, "width": 0, "height": 0}, disabled

    run_scenarios()

    with subtest("SWAYSOCK takes precedence over I3SOCK"):
        i3_ws("--ws", "5", env=f"SWAYSOCK={SOCKET} I3SOCK=/nonexistent")
        check("205")
        _, stderr = i3_ws(
            "--ws", "6", env=f"SWAYSOCK=/nonexistent I3SOCK={SOCKET}", status=1
        )
        assert "cannot connect to /nonexistent" in stderr, stderr
        check("205")

    with subtest("I3SOCK without SWAYSOCK"):
        i3_ws("--ws", "6", env=f"I3SOCK={SOCKET}")
        check("206")

    with subtest("an empty SWAYSOCK counts as unset"):
        i3_ws("--ws", "7", env=f"SWAYSOCK= I3SOCK={SOCKET}")
        check("207")

    with subtest("no socket"):
        # i3 isn't installed, so `i3 --get-socketpath` fails too.
        _, stderr = i3_ws("--ws", "8", env="SWAYSOCK= I3SOCK=", status=1)
        assert "i3-ws: no i3 or sway IPC socket found" in stderr, stderr
        check("207")
  '';
}
