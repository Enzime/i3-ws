{ lib, ... }:

{
  name = "i3-ws-i3";

  nodes.machine =
    { i3-ws, pkgs, ... }:
    let
      # Four outputs, fake-0 to fake-3, that aren't listed in the order i3-ws
      # numbers them. fake-3 and fake-1 share x and width, so y decides.
      # fake-2 and fake-0 share x, so width decides before y.
      i3Config = pkgs.writeText "i3-config" ''
        fake-outputs 1024x768+1024+0,1024x768+0+768,800x600+1024+768,1024x768+0+0
        ipc-socket /run/i3/ipc.sock
        focus_follows_mouse no
        mouse_warping none
      '';
    in
    {
      users.users.alice.isNormalUser = true;

      environment.systemPackages = [
        i3-ws
        pkgs.i3
      ];

      systemd.services.xvfb = {
        wantedBy = [ "multi-user.target" ];
        serviceConfig = {
          User = "alice";
          ExecStart = "${lib.getExe' pkgs.xvfb "Xvfb"} :0 -screen 0 2048x1536x24 -nolisten tcp";
        };
      };

      # Started by the test script once Xvfb is listening.
      systemd.services.i3 = {
        environment.DISPLAY = ":0";
        # For the programs the test starts with i3's `exec`.
        path = [
          i3-ws
          pkgs.i3
          pkgs.xterm
        ];
        serviceConfig = {
          User = "alice";
          RuntimeDirectory = "i3";
          ExecStart = "${lib.getExe pkgs.i3} -c ${i3Config}";
        };
      };
    };

  testScript = ''
    # i3-ws sorts outputs by x, then width, then y, then height.
    OUTPUTS = ["fake-3", "fake-1", "fake-2", "fake-0"]
    LAYOUT = {
        "fake-0": (1024, 0, 1024, 768),
        "fake-1": (0, 768, 1024, 768),
        "fake-2": (1024, 768, 800, 600),
        "fake-3": (0, 0, 1024, 768),
    }
    # i3-ws runs `i3 --get-socketpath`, which needs DISPLAY.
    ENV = "DISPLAY=:0"
    WINDOW = "xterm -T {title} -e sleep infinity"


    def ipc(msg_type: str, payload: str = ""):
        command = f"i3-msg -s /run/i3/ipc.sock -t {msg_type}"
        if payload:
            command += f" -- {shlex.quote(payload)}"
        return json.loads(machine.succeed(command))


    def wm_exec(command: str) -> None:
        ipc("command", f"exec --no-startup-id {command}")


    ${builtins.readFile ./scenarios.py}

    machine.wait_for_file("/tmp/.X11-unix/X0")
    machine.succeed("systemctl start i3")
    machine.wait_until_succeeds("i3-msg -s /run/i3/ipc.sock -t get_version")
    machine.wait_until_succeeds(
        "setpriv --reuid=alice --regid=users --init-groups env DISPLAY=:0 i3 --get-socketpath"
    )
    wait_for_outputs()

    run_scenarios()

    with subtest("i3-ws without DISPLAY"):
        # `i3 --get-socketpath` fails, and i3ipc++ throws an exception that
        # nothing catches.
        _, stderr = i3_ws("--ws", "1", env="", status=134)
        assert "terminate called" in stderr, stderr
        check("210")
  '';
}
