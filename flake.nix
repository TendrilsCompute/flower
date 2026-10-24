{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

  outputs =
    { nixpkgs, ... }:
    let
      forSystems = f: nixpkgs.lib.genAttrs [
        "x86_64-linux"
        "aarch64-darwin"
      ] (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forSystems (pkgs: rec {
        flower = pkgs.rustPlatform.buildRustPackage {
          pname = "flower";
          version = "0.0.1";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
        };
        default = flower;
      });

      lib.flow = import ./nix/flow.nix;
    };
}
