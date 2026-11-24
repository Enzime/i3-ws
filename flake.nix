{
  inputs.self.submodules = true;

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
        {
          config,
          lib,
          pkgs,
          ...
        }:
        {
          packages.default = pkgs.stdenv.mkDerivation {
            pname = "i3-ws";
            version = inputs.self.shortRev or inputs.self.dirtyShortRev;

            src = inputs.self;

            buildInputs = builtins.attrValues { inherit (pkgs) i3 jsoncpp libsigcxx_2_0; };

            nativeBuildInputs = builtins.attrValues { inherit (pkgs) cmake pkg-config; };

            # Both `CMakeLists.txt` files declare a minimum below 3.5, which CMake 4 rejects
            cmakeFlags = [ (lib.cmakeFeature "CMAKE_POLICY_VERSION_MINIMUM" "3.5") ];

            meta.mainProgram = "i3-ws";
          };

          checks.package = config.packages.default;

          checks.vm-i3 = pkgs.testers.runNixOSTest {
            imports = [ ./nixos-tests/i3.nix ];
            node.specialArgs.i3-ws = config.packages.default;
          };
        };
    };
}
