{
  description = "nixvis — interactive package explorer and dependency visualizer for the Nix ecosystem";

  # Binary cache configuration (Cachix).
  # To enable: create a free cache at https://www.cachix.org named "nixvis",
  # then replace the public-key placeholder below with your real key.
  nixConfig = {
    extra-substituters = "https://nixvis.cachix.org";
    extra-trusted-public-keys = "nixvis.cachix.org-1:PLACEHOLDER_REPLACE_WITH_REAL_KEY=";
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        manifest = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      in
      {
        packages = {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = manifest.package.name;
            version = manifest.package.version;
            src = ./.;
            buildFeatures = [ "web" ];
            cargoLock = {
              lockFile = ./Cargo.lock;
            };
            nativeBuildInputs = with pkgs; [ pkg-config ];
            buildInputs = with pkgs; [
              openssl
            ] ++ lib.optionals stdenv.hostPlatform.isDarwin [
              libiconv
              Security
            ];
            meta = with pkgs.lib; {
              description = manifest.package.description;
              license = licenses.gpl3Plus;
              maintainers = [ "stefan-hacks" ];
              mainProgram = "nixvis";
            };
          };
        };

        apps.default = flake-utils.lib.mkApp {
          drv = self.packages.${system}.default;
        };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
            rust-analyzer
            pkg-config
            openssl
          ];
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };
      });
}
