# nixvis ❄️

> Interactive package explorer and dependency visualiser for the Nix ecosystem.

**nixvis** indexes your Nix packages (via `nix search`, `nix derivation show`, or a pre-built JSON dump) and exposes them through both a **keyboard-first terminal UI** and a **web-based single-page application** with a force-directed Canvas graph engine.

Whether you're debugging a closure size explosion, auditing reverse dependencies, or just browsing `nixpkgs`, nixvis gives you the big picture.

---

## Table of Contents

1. [Features](#features)
2. [Installation](#installation)
3. [Usage](#usage)
4. [Keybindings](#keybindings)
5. [Web Mode](#web-mode)
6. [Architecture](#architecture)
7. [Theming](#theming)
8. [Data Format](#data-format)
9. [Contributing](#contributing)
10. [Licence](#licence)

---

## Features

- 🔎 **Fuzzy search** across package names, attribute paths, and descriptions (powered by a nucleo-style scoring heuristic).
- 📊 **Force-directed dependency graph** rendered on HTML5 Canvas with pan, zoom, and hover interaction.
- 🌲 **Dependency trees** (forward and reverse) with BFS depth control.
- 🧊 **Nix-native concepts** throughout: attribute paths, flake inputs, propagated native build inputs, store paths, and NixOS / Home-Manager options.
- 🎨 **Four themes**: Catppuccin Mocha (default), Dark, Nix Snow, and Grayscale (accessibility / `NO_COLOR`).
- 🌐 **Web SPA** served by an embedded Axum server (`--web` flag).
- 🖥️ **TUI** built with [ratatui](https://github.com/ratatui/ratatui) for fast keyboard-driven exploration.
- 💾 **Cached index**: serialises to gzipped JSON so subsequent starts are instant.
- 📦 **Multi-flake support**: packages from `nixpkgs`, `home-manager`, and custom flakes coexist in one index.
- ⚙️ **NixOS options browser**: search and inspect NixOS configuration options (`services.openssh.enable`, etc.).
- 🏠 **Home-Manager options browser**: search Home-Manager configuration options (`programs.git.enable`, etc.).
- ❄️ **Nix branding**: snowflake icons, Catppuccin Mocha colors, NixOS-inspired aesthetics.

---

## Installation

### Prerequisites

- **Rust** ≥ 1.78
- **Nix** (for live indexing from a Nix store)
- **A modern browser** (for web mode)

### Via Cargo

```bash
git clone https://github.com/stefan-hacks/nixvis.git
cd nixvis
cargo install --path .
```

### Via Nix

```bash
nix run github:stefan-hacks/nixvis
```

> **Tip — enable the binary cache** so you never compile from source:
> ```bash
> nix run nixpkgs#cachix -- use nixvis
> ```
> After that, `nix run github:stefan-hacks/nixvis` resolves in **under 1 second** instead of 1–2 minutes.

### Via Flake

Add to your `flake.nix` inputs:

```nix
{
  inputs.nixvis.url = "github:stefan-hacks/nixvis";

  outputs = { self, nixpkgs, nixvis, ... }: {
    # For nixosConfiguration:
    # environment.systemPackages = [ nixvis.packages.${pkgs.system}.default ];
  };
}
```

---

## Usage

### TUI Mode (default)

```bash
# Automatically discover nixpkgs and build the index.
nixvis

# Force a full re-index (useful after `nix-channel --update`).
nixvis --rebuild

# Specify a custom index file (bypasses live indexing).
nixvis --index ~/nix-index.json.gz
```

### Web Mode

```bash
# Start the embedded Axum server on the default port (8787).
nixvis --web

# Custom port:
nixvis --web --port 3000
```

Then open `http://localhost:8787` in your browser.  The web frontend is a fully self-contained SPA: no bundler, no npm, no build step.

### Indexing manually

If you want to pre-build the index for CI or sharing:

```bash
# Generate from nixpkgs
nix search nixpkgs --json "" > nix-index.json

# Or use the bundled demo data
cp data/nix-index.json /tmp/nix-index.json
nixvis --index /tmp/nix-index.json
```

---

## Keybindings

### Global

| Key | Action |
|-----|--------|
| `?` | Toggle help modal |
| `t` | Cycle theme forward |
| `T` | Cycle theme backward |
| `q` / `Ctrl-C` | Quit |

### Search

| Key | Action |
|-----|--------|
| `/` or `s` | Focus search input |
| `Enter` | Submit search |
| `Esc` | Clear search / unfocus |
| `↑` / `↓` | Navigate autocomplete suggestions |

### Results List

| Key | Action |
|-----|--------|
| `j` / `↓` | Next result |
| `k` / `↑` | Previous result |
| `g` | Jump to top |
| `G` | Jump to bottom |
| `PgDn` / `PgUp` | Page down / up |
| `Enter` or `l` | Open selected package details |
| `d` | Open dependency tree |
| `r` | Open reverse-dependency tree |
| `v` | Open graph view |
| `o` | Open homepage in browser |

### Detail Panel

| Key | Action |
|-----|--------|
| `Esc` or `h` | Back to list |
| `j` / `↓` | Scroll down |
| `k` / `↑` | Scroll up |
| `i` | Info tab |
| `d` | Dependencies tab |
| `r` | Reverse dependencies tab |
| `l` / `h` | Expand / collapse tree nodes |

### Graph View

| Key | Action |
|-----|--------|
| `g` | Reset view (fit to screen) |
| `+` / `=` | Increase depth |
| `-` | Decrease depth |
| `d` | Show forward dependencies |
| `Shift+D` | Show reverse dependencies |

---

## Web Mode

The web frontend (`web/`) is a zero-dependency single-page application:

| File | Responsibility |
|------|----------------|
| `index.html` | Semantic shell, DOM skeleton, accessibility landmarks |
| `style.css`  | Catppuccin Mocha theming via CSS custom properties, responsive grid, dark-mode-first |
| `app.js`     | Index loading, fuzzy search, UI state management, detail tabs, theme switching |
| `graph.js`   | Canvas force-directed graph engine: Verlet integration, pan/zoom, hover highlighting |
| `favicon.svg`| Snowflake icon |

### Web-only shortcuts

- **Pan**: click-drag on the canvas
- **Zoom**: mouse wheel (or pinch on touch)
- **Hover**: highlights a node and shows its label
- **Click**: focuses a node (accent colour)

### Responsive breakpoints

| Width | Layout |
|-------|--------|
| > 1100 px | Three columns: search, graph, detail |
| 720–1100 px | Two columns; detail is a slide-out drawer |
| < 720 px | Single column; stacked panels |

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         nixvis binary                           │
├─────────────────────────────────────────────────────────────────┤
│  CLI (clap)                                                     │
│   ├── --web       → Axum server + embedded static assets        │
│   ├── --rebuild   → Force re-index from nix search              │
│   └── --index     → Load pre-generated nix-index.json           │
├─────────────────────────────────────────────────────────────────┤
│  Indexer                                                        │
│   ├── nix search --json (live)                                  │
│   ├── nix derivation show (live)                                │
│   └── data/nix-index.json (embedded fallback)                   │
├─────────────────────────────────────────────────────────────────┤
│  In-Memory Index (index.rs)                                     │
│   ├── packages: Vec<Package>                                   │
│   ├── names: HashMap<Arc<str>, u32>                            │
│   ├── dependents: Vec<Vec<u32>>                                │
│   └── module_neighbors: Vec<Vec<u32>>                         │
├─────────────────────────────────────────────────────────────────┤
│  Search Engine (search.rs)                                      │
│   └── nucleo-matcher for fuzzy substring scoring                │
├─────────────────────────────────────────────────────────────────┤
│  Graph Engine (graph.rs)                                        │
│   └── BFS projection + layout hints for TUI / web               │
├─────────────────────────────────────────────────────────────────┤
│  TUI (ratatui)              │  Web (Axum + SPA)                 │
│   ├── ui/list.rs            │   ├── GET / (index.html)          │
│   ├── ui/detail.rs          │   ├── GET /style.css              │
│   ├── ui/graph.rs           │   ├── GET /app.js                 │
│   └── theme.rs              │   ├── GET /graph.js               │
│                             │   └── GET /data/nix-index.json    │
└─────────────────────────────────────────────────────────────────┘
```

### Key modules

| Module | Purpose |
|--------|---------|
| `model.rs` | Serde types for `nix search --json`, `nix flake metadata`, NixOS / HM options |
| `index.rs` | In-memory index with interned strings, BFS traversal, reverse-dependency resolution |
| `search.rs` | Fuzzy search over names, attributes, and descriptions |
| `graph.rs` | Graph projections (BFS with depth + budget caps) |
| `theme.rs` | Catppuccin Mocha, Dark, Snow, and Grayscale palettes |
| `app.rs` | Application state machine: input handling, tab switching, focus management |
| `cache.rs` | Gzipped JSON read/write and nixpkgs commit detection |

---

## Theming

Themes are defined in `src/theme.rs` (TUI) and mirrored in `web/style.css` (web).

### Available themes

1. **catppuccin-mocha** (default) — warm dark, low eye-strain, Nix-blue accents
2. **dark** — cooler, higher contrast
3. **nix-snow** — icy blues, deeper navy backgrounds
4. **grayscale** — monochrome, respects `NO_COLOR`

### Switching

- **TUI**: press `t` (forward) or `T` (backward)
- **Web**: press `t` or click the ◐ button in the top bar

### Adding a new theme

1. Add a `Theme` constant in `src/theme.rs`.
2. Register it in the `THEMES` array.
3. Add a corresponding `body[data-theme="..."]` block in `web/style.css`.

---

## Data Format

The on-disk index is gzipped JSON matching the `IndexDoc` schema:

```json
{
  "header": {
    "schema": 1,
    "nixpkgs_commit": "abc123def456",
    "generated_ms": "1726621200000",
    "package_count": 42000,
    "channel": "nixpkgs-unstable"
  },
  "packages": [
    {
      "id": 0,
      "name": "hello",
      "attribute": "nixpkgs#hello",
      "version": "2.12.1",
      "synopsis": "...",
      "description": "...",
      "homepage": "...",
      "licenses": ["GPL-3.0-or-later"],
      "file": ["nixpkgs/.../default.nix", 42],
      "inputs": [1, 2],
      "propagated_inputs": [],
      "native_inputs": [3, 4],
      "store_path": "/nix/store/...",
      "installed": true,
      "flake": "nixpkgs"
    }
  ]
}
```

Fields:

| Field | Meaning |
|-------|---------|
| `id` | Stable numeric ID (must be contiguous) |
| `name` | Human-readable package name |
| `attribute` | Nix attribute path (e.g. `nixpkgs#hello`) |
| `inputs` | Runtime / build dependencies (by name, resolved to IDs at load time) |
| `propagated_inputs` | Propagated dependencies pulled into consumer closures |
| `native_inputs` | Native build inputs (compilers, make, etc.) |
| `store_path` | Absolute Nix store path when installed |
| `installed` | Whether this package is present in a profile |
| `flake` | Origin flake (e.g. `nixpkgs`, `home-manager`) |

---

## Contributing

Contributions are welcome!  Please open an issue or PR on [GitHub](https://github.com/stefan-hacks/nixvis).

### Development workflow

```bash
# Clone
git clone https://github.com/stefan-hacks/nixvis.git
cd nixvis

# Run TUI in debug mode
cargo run

# Run tests
cargo test

# Run web server with live reload (separate terminal)
cargo run --features web -- --web --port 8787

# Format + lint
cargo fmt --check
cargo clippy --all-features
```

### Code style

- `rustfmt` for formatting.
- `clippy` linting is enforced in CI.
- All modules start with a `//!` doc comment explaining their role.
- Nix-specific terminology is preferred (attribute path, flake input, propagated native build input, store path).

---

## Licence

GPL-3.0-or-later © 2024 Lin <stefan-hacks>

See [LICENCE](./LICENCE) for the full text.
