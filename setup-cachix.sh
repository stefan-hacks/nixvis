#!/usr/bin/env bash
# setup-cachix.sh — One-time setup for the nixvis binary cache
# ==============================================================
#
# Run this script once on your development machine to:
#   1. Install the Cachix CLI.
#   2. Create (or reuse) a cache named "nixvis".
#   3. Print the auth token you need to paste into GitHub Secrets.
#   4. Show the next steps.
#
# Usage:
#   bash setup-cachix.sh
#
# After this script runs:
#   1. Copy the printed CACHIX_AUTH_TOKEN into GitHub:
#      Repo → Settings → Secrets and variables → Actions → New repository secret
#   2. Replace PLACEHOLDER in flake.nix with the real public key printed below.
#   3. Push to main; GitHub Actions will build and cache the binary.

set -euo pipefail

echo "=== nixvis Cachix setup ==="
echo

# 1. Install cachix if missing
if ! command -v cachix &>/dev/null; then
    echo "❄  Installing Cachix CLI..."
    nix-env -iA nixpkgs.cachix || nix-shell -p cachix
else
    echo "✅ Cachix CLI already installed: $(cachix --version)"
fi

# 2. Log in (opens browser for OAuth)
echo
echo "❄  Logging into Cachix (opens browser if not already authenticated)..."
cachix login

# 3. Create the cache (idempotent — safe to re-run)
echo
echo "❄  Creating cache 'nixvis' (free, public read)..."
cachix create nixvis || echo "   (cache already exists — continuing)"

# 4. Get auth token
echo
echo "❄  Generating auth token for CI..."
AUTH_TOKEN=$(cachix authtoken nixvis 2>/dev/null || echo "")
if [ -z "$AUTH_TOKEN" ]; then
    echo "   ⚠️  Could not auto-generate token. Go to https://nixvis.cachix.org/settings"
    echo "      and create a token manually, then paste it into GitHub Secrets as CACHIX_AUTH_TOKEN."
else
    echo "   🔑 CACHIX_AUTH_TOKEN (paste into GitHub Secrets):"
    echo "      $AUTH_TOKEN"
fi

# 5. Show public key
echo
echo "❄  Cache public key (for flake.nix):"
cachix show nixvis 2>/dev/null | grep "Public key" || echo "   Run: cachix show nixvis"

echo
echo "=== Next steps ==="
echo "1. Go to https://github.com/stefan-hacks/nixvis/settings/secrets/actions"
echo "2. Click 'New repository secret' → Name: CACHIX_AUTH_TOKEN → Value: (token above)"
echo "3. Replace PLACEHOLDER in flake.nix with the real public key."
echo "4. git add -A && git commit -m 'Setup Cachix binary cache' && git push origin main"
echo "5. GitHub Actions will automatically build + cache on the next push."
echo
echo "After that, anyone can run:"
echo "   nix run github:stefan-hacks/nixvis -- web"
echo "and get an instant binary (no compilation)."
