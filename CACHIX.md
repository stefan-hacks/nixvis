# nixvis — Nix binary cache (Cachix)
# =================================
#
# This file configures the Cachix binary cache so that anyone running
# `nix run github:stefan-hacks/nixvis` gets a pre-built binary instead of
# compiling Rust dependencies from source.
#
# Cachix name: nixvis
# Public URL:   https://nixvis.cachix.org
#
# Setup (done once per machine):
#   nix-env -iA nixpkgs.cachix          # or: nix-shell -p cachix
#   cachix use nixvis                   # adds trusted-public-keys + substituters
#
# Push from CI (GitHub Actions):
#   See .github/workflows/build.yml — pushes after every successful build on main.
#
# Manual push (after local build):
#   nix build .#
#   cachix push nixvis result
#
# Binary cache details:
#   - Public read: yes  (no auth needed to download)
#   - Public write: no  (only CI and authorized maintainers can push)
#   - Compression: zstd
#   - Signing: ed25519 (public key below)
#
# Trusted public key (add to /etc/nix/nix.conf or via `cachix use`):
#   nixvis.cachix.org-1:<64-char-key>
#
# If you don't want to install cachix globally, add these flags to any nix command:
#   --option substituters 'https://cache.nixos.org https://nixvis.cachix.org'
#   --option trusted-public-keys 'cache.nixos.org-1:... nixvis.cachix.org-1:...'
