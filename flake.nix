{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

  inputs.flake-parts.url = "github:hercules-ci/flake-parts";
  inputs.flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";

  outputs =
    inputs:
    inputs.flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "aarch64-linux"
        "x86_64-linux"
      ];

      perSystem =
        { config, pkgs, ... }:
        {
          packages.default = pkgs.callPackage ./package.nix { };

          checks.package = config.packages.default;

          checks.vm-i3 = pkgs.testers.runNixOSTest {
            imports = [ ./nixos-tests/i3.nix ];
            node.specialArgs.i3-ws = config.packages.default;
          };
        };
    };
}
