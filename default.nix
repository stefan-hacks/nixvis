{ pkgs ? import <nixpkgs> {} }:

pkgs.rustPlatform.buildRustPackage {
  pname = "nixvis";
  version = "0.1.0";
  src = ./.;
  buildFeatures = [ "web" ];
  cargoLock.lockFile = ./Cargo.lock;
  nativeBuildInputs = [ pkgs.pkg-config ];
  buildInputs = [ pkgs.openssl ];
  meta = with pkgs.lib; {
    description = "Interactive package explorer and dependency visualizer for the Nix ecosystem";
    license = licenses.gpl3Plus;
    maintainers = [ "stefan-hacks" ];
    mainProgram = "nixvis";
  };
}