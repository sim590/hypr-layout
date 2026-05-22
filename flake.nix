{
  description = "A CLI tool to launch and arrange tiled window layouts in Hyprland using a simple DSL";

  inputs = {
    nixpkgs.url     = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname   = "hypr-layout";
          version = "0.1.0";
          src     = ./.;

          cargoLock.lockFile = ./Cargo.lock;

          meta = with pkgs.lib; {
            description = "A CLI tool to launch and arrange tiled window layouts in Hyprland using a simple DSL";
            homepage    = "https://github.com/sim590/hypr-layout";
            license     = licenses.gpl3Only;
            maintainers = [{ name = "Simon Désaulniers"; github = "sim590"; }];
            mainProgram = "hypr-layout";
            platforms   = platforms.linux;
          };
        };

        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            rustc
            cargo
            rust-analyzer
            clippy
            rustfmt
          ];
        };
      });
}
