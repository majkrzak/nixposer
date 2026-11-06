{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";
    crane.url = "github:ipetkov/crane";
  };
  outputs =
    inputs:
    inputs.flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      imports = [ inputs.treefmt-nix.flakeModule ];
      perSystem =
        { lib, pkgs, ... }:
        let
          craneLib = inputs.crane.mkLib pkgs;
        in
        {
          packages.default = craneLib.buildPackage {
            src = lib.fileset.toSource {
              root = ./.;
              fileset = lib.fileset.unions [
                (craneLib.fileset.commonCargoSources ./.)
                (lib.fileset.maybeMissing ./schema)
              ];
            };
          };
          devShells.default = craneLib.devShell { };
          treefmt.programs = {
            nixfmt.enable = true;
            rustfmt.enable = true;
            flake-edit.enable = true;
            mdformat.enable = true;
            taplo.enable = true;
            yamlfmt.enable = true;
            jsonfmt.enable = true;
          };
        };
    };
}
