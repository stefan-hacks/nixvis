/**
 * nixvis — Application Logic (app.js)
 * ===================================
 * Single-page application controller.  Responsibilities:
 *
 * 1. Index loading — fetch data/nix-index.json and normalise it into an
 *    in-memory lookup table.
 * 2. Search — fuzzy substring matching with a simple scoring heuristic inspired
 *    by nucleo (consecutive-letter bonus, start-of-word bonus).
 * 3. UI state — results list, selected package, detail tabs, graph direction,
 *    graph depth, and active theme.
 * 4. Theme switching — cycle through Catppuccin Mocha → dark → nix-snow →
 *    grayscale via body[data-theme].
 * 5. Keyboard shortcuts — global keydown listener dispatching to focused
 *    widgets or global commands.
 * 6. Graph bridge — pass projection data (nodes + edges) to graph.js when
 *    the selected package changes or graph parameters change.
 *
 * Nix-specific terminology is used throughout: attribute path, flake input,
 * propagated native build inputs, store path, NixOS options, Home-Manager.
 */

(function () {
  'use strict';

  // =====================================================================
  //  CONSTANTS
  // =====================================================================

  /** Maximum results rendered in the list before truncation. */
  const MAX_RESULTS = 500;

  /** Maximum autocomplete suggestions. */
  const MAX_SUGGESTIONS = 8;

  /** Debounce delay for search input (ms). */
  const SEARCH_DEBOUNCE_MS = 40;

  /** Theme cycle order.  Must match CSS body[data-theme] selectors. */
  const THEMES = ['catppuccin-mocha', 'dark', 'nix-snow', 'grayscale'];

  // =====================================================================
  //  STATE
  // =====================================================================

  /**
   * Global application state.  All UI reads from / writes to this object so
   * re-renders are deterministic and single-source-of-truth.
   */
  const state = {
    /** Parsed index document: { header, packages[] } */
    index: null,
    /** Flat array of all package objects for fast iteration. */
    packages: [],
    /** Map from package name → package object. */
    byName: new Map(),
    /** Map from attribute path → package object. */
    byAttr: new Map(),
    /** Current search query string. */
    query: '',
    /** Array of package objects matching the current query. */
    results: [],
    /** Index into results[] of the currently selected (highlighted) item. */
    selectedIdx: 0,
    /** Currently displayed package in the detail panel, or null. */
    activePkg: null,
    /** Detail tab name: 'info' | 'deps' | 'reverse' | 'flake'. */
    detailTab: 'info',
    /** Graph direction: 'deps' | 'reverse'. */
    graphDir: 'deps',
    /** Graph BFS depth (1–8). */
    graphDepth: 2,
    /** Current theme index into THEMES. */
    themeIdx: 0,
    /** Whether the help modal is open. */
    helpOpen: false,
    /** Whether the detail panel is open (mobile drawer). */
    detailOpen: false,
    /** Autocomplete suggestion index (-1 = none focused). */
    suggestIdx: -1,
    /** Debounce timer handle. */
    debounceTimer: null,
  };

  // =====================================================================
  //  DOM REFERENCES
  //  We cache all queried elements here so we never re-query during hot paths.
  // =====================================================================

  const $ = (sel) => document.querySelector(sel);
  const $$ = (sel) => document.querySelectorAll(sel);

  const els = {
    searchInput:  $('#search-input'),
    searchSuggest: $('#search-suggest'),
    resultsList:  $('#search-results'),
    resultsMeta:  $('#results-meta'),
    resultsCount: $('#results-meta .results-meta__count'),
    commitHash:   $('#commit-hash'),
    detailEmpty:  $('#detail-empty'),
    detailContent:$('#detail-content'),
    detailName:   $('#detail-name'),
    detailBadges: $('#detail-badges'),
    detailBody:   $('#detail-body'),
    detailTabs:   $$('.detail-tab'),
    graphCanvas:  $('#graph-canvas'),
    graphStatus:  $('#graph-status'),
    graphDepth:   $('#graph-depth'),
    graphDepthVal:$('#graph-depth-val'),
    btnTheme:     $('#btn-theme'),
    btnHelp:      $('#btn-help'),
    helpModal:    $('#help-modal'),
    btnHelpClose: $('#btn-help-close'),
    btnGraphDeps: $('#btn-graph-deps'),
    btnGraphRev:  $('#btn-graph-rev'),
    btnGraphReset:$('#btn-graph-reset'),
    toast:        $('#toast'),
    panelDetail:  $('.panel--detail'),
  };

  // =====================================================================
  //  INDEX LOADING
  // =====================================================================

  /**
   * Fetch the gzipped (or plain) JSON index from data/nix-index.json.
   * If the server sends Content-Encoding: gzip the browser decompresses
   * automatically; otherwise we assume plain JSON for local development.
   */
  async function loadIndex () {
    try {
      const res = await fetch('data/nix-index.json');
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const doc = await res.json();
      state.index = doc;
      ingestIndex(doc);
      showToast(`Loaded ${state.packages.length.toLocaleString()} packages · ${doc.header.nixpkgs_commit || 'local'}`);
    } catch (err) {
      console.error('Failed to load index:', err);
      showToast('Failed to load index — using demo data', 'error');
      loadDemoData();
    }
  }

  /**
   * Transform the raw IndexDoc into optimised lookup structures.
   * @param {Object} doc — { header, packages[] }
   */
  function ingestIndex (doc) {
    state.packages = doc.packages || [];
    state.byName.clear();
    state.byAttr.clear();
    for (const pkg of state.packages) {
      if (pkg.name) state.byName.set(pkg.name, pkg);
      if (pkg.attribute) state.byAttr.set(pkg.attribute, pkg);
    }
    if (doc.header) {
      els.commitHash.textContent = (doc.header.nixpkgs_commit || '').slice(0, 12);
    }
    els.resultsCount.textContent = `${state.packages.length.toLocaleString()} packages`;
    // Show first 20 packages initially so the list isn't empty.
    state.results = state.packages.slice(0, MAX_RESULTS);
    renderResults();
  }

  /**
   * Fallback demo data when the real index is unavailable (e.g. opened
   * directly in a browser without a server).
   */
  function loadDemoData () {
    const demo = generateDemoData();
    ingestIndex(demo);
  }

  /**
   * Generate a small synthetic Nix index for demonstration purposes.
   * Mirrors the Rust IndexDoc schema exactly.
   */
  function generateDemoData () {
    const packages = [
      {
        id: 0, name: 'hello', attribute: 'nixpkgs#hello',
        version: '2.12.1',
        synopsis: 'A program that produces a familiar, friendly greeting',
        description: 'GNU Hello prints a friendly greeting. It serves as a canonical example of a GNU package and is often used to test Nix builds.',
        homepage: 'https://www.gnu.org/software/hello/',
        licenses: ['GPL-3.0-or-later'],
        file: ['nixpkgs/pkgs/applications/misc/hello/default.nix', 42],
        inputs: ['glibc', 'coreutils'],
        propagated_inputs: [], native_inputs: ['gcc', 'make'],
        store_path: '/nix/store/…-hello-2.12.1',
        installed: true, flake: 'nixpkgs',
      },
      {
        id: 1, name: 'glibc', attribute: 'nixpkgs#glibc',
        version: '2.39',
        synopsis: 'GNU C Library',
        description: 'The GNU C Library project provides the core libraries for the GNU system and GNU/Linux systems.',
        homepage: 'https://www.gnu.org/software/libc/',
        licenses: ['LGPL-2.1-or-later'],
        file: ['nixpkgs/pkgs/development/libraries/glibc/default.nix', 128],
        inputs: ['linux-headers'],
        propagated_inputs: [], native_inputs: ['gcc'],
        store_path: '/nix/store/…-glibc-2.39',
        installed: true, flake: 'nixpkgs',
      },
      {
        id: 2, name: 'coreutils', attribute: 'nixpkgs#coreutils',
        version: '9.4',
        synopsis: 'Basic file, shell and text manipulation utilities',
        description: 'The GNU Core Utilities are the basic file, shell and text manipulation utilities of the GNU operating system.',
        homepage: 'https://www.gnu.org/software/coreutils/',
        licenses: ['GPL-3.0-or-later'],
        file: ['nixpkgs/pkgs/tools/misc/coreutils/default.nix', 88],
        inputs: ['glibc', 'acl', 'attr'],
        propagated_inputs: [], native_inputs: ['gcc', 'perl'],
        store_path: '', installed: false, flake: 'nixpkgs',
      },
      {
        id: 3, name: 'nix', attribute: 'nixpkgs#nix',
        version: '2.18.2',
        synopsis: 'The purely functional package manager',
        description: 'Nix is a powerful package manager for Linux and other Unix systems that makes package management reliable and reproducible.',
        homepage: 'https://nixos.org/',
        licenses: ['LGPL-2.1'],
        file: ['nixpkgs/pkgs/tools/package-management/nix/default.nix', 256],
        inputs: ['boost', 'editline', 'libsodium', 'openssl', 'curl'],
        propagated_inputs: ['nixpkgs'],
        native_inputs: ['autoconf', 'automake', 'pkg-config'],
        store_path: '/nix/store/…-nix-2.18.2',
        installed: true, flake: 'nixpkgs',
      },
      {
        id: 4, name: 'home-manager', attribute: 'home-manager#home-manager',
        version: '24.05',
        synopsis: 'Nix-based user environment configurator',
        description: 'Home Manager provides a flexible way of managing a user environment using the Nix package manager.',
        homepage: 'https://github.com/nix-community/home-manager',
        licenses: ['MIT'],
        file: ['home-manager/modules/default.nix', 1],
        inputs: ['nixpkgs'],
        propagated_inputs: [], native_inputs: [],
        store_path: '', installed: false, flake: 'home-manager',
      },
      {
        id: 5, name: 'firefox', attribute: 'nixpkgs#firefox',
        version: '125.0.1',
        synopsis: 'Mozilla Firefox web browser',
        description: 'A free and open-source web browser developed by the Mozilla Foundation.',
        homepage: 'https://www.mozilla.org/firefox/',
        licenses: ['MPL-2.0'],
        file: ['nixpkgs/pkgs/applications/networking/browsers/firefox/default.nix', 512],
        inputs: ['gtk3', 'dbus', 'libx11', 'nss', 'nspr'],
        propagated_inputs: ['ffmpeg'],
        native_inputs: ['rustc', 'cargo', 'llvm'],
        store_path: '', installed: false, flake: 'nixpkgs',
      },
      {
        id: 6, name: 'neovim', attribute: 'nixpkgs#neovim',
        version: '0.9.5',
        synopsis: 'Hyperextensible Vim-based text editor',
        description: 'Neovim is a refactor, and sometimes redactor, in the tradition of Vim.',
        homepage: 'https://neovim.io/',
        licenses: ['Apache-2.0'],
        file: ['nixpkgs/pkgs/applications/editors/neovim/default.nix', 200],
        inputs: ['luajit', 'libuv', 'msgpack-c', 'tree-sitter'],
        propagated_inputs: [], native_inputs: ['cmake', 'ninja'],
        store_path: '/nix/store/…-neovim-0.9.5',
        installed: true, flake: 'nixpkgs',
      },
      {
        id: 7, name: 'nixpkgs-fmt', attribute: 'nixpkgs#nixpkgs-fmt',
        version: '1.3.0',
        synopsis: 'Nix code formatter for nixpkgs',
        description: 'An opinionated formatter for Nix source code, optimised for nixpkgs conventions.',
        homepage: 'https://github.com/nix-community/nixpkgs-fmt',
        licenses: ['Apache-2.0'],
        file: ['nixpkgs/pkgs/tools/nix/nixpkgs-fmt/default.nix', 24],
        inputs: ['nix'],
        propagated_inputs: [], native_inputs: ['rustPlatform'],
        store_path: '', installed: false, flake: 'nixpkgs',
      },
      {
        id: 8, name: 'docker', attribute: 'nixpkgs#docker',
        version: '25.0.3',
        synopsis: 'Pack, ship and run any application as a lightweight container',
        description: 'Docker is a platform for developers and sysadmins to develop, deploy, and run applications with containers.',
        homepage: 'https://www.docker.com/',
        licenses: ['Apache-2.0'],
        file: ['nixpkgs/pkgs/applications/virtualization/docker/default.nix', 300],
        inputs: ['containerd', 'runc', 'tini', 'iptables'],
        propagated_inputs: ['libseccomp'],
        native_inputs: ['go', 'glibc'],
        store_path: '', installed: false, flake: 'nixpkgs',
      },
      {
        id: 9, name: 'git', attribute: 'nixpkgs#git',
        version: '2.44.0',
        synopsis: 'Distributed version control system',
        description: 'Git is a free and open source distributed version control system designed to handle everything from small to very large projects.',
        homepage: 'https://git-scm.com/',
        licenses: ['GPL-2.0'],
        file: ['nixpkgs/pkgs/applications/version-management/git/default.nix', 180],
        inputs: ['curl', 'openssl', 'expat', 'zlib'],
        propagated_inputs: [], native_inputs: ['asciidoc', 'xmlto'],
        store_path: '/nix/store/…-git-2.44.0',
        installed: true, flake: 'nixpkgs',
      },
    ];

    // Resolve string inputs into numeric IDs by looking up names.
    const byName = new Map(packages.map(p => [p.name, p.id]));
    for (const pkg of packages) {
      pkg.inputs = pkg.inputs.map(n => byName.get(n)).filter(v => v !== undefined);
      pkg.propagated_inputs = pkg.propagated_inputs.map(n => byName.get(n)).filter(v => v !== undefined);
      pkg.native_inputs = pkg.native_inputs.map(n => byName.get(n)).filter(v => v !== undefined);
    }

    return {
      header: {
        schema: 1,
        nixpkgs_commit: '24.05-demo',
        generated_ms: Date.now().toString(),
        package_count: packages.length,
        channel: 'nixpkgs-unstable',
      },
      packages,
    };
  }

  // =====================================================================
  //  SEARCH
  // =====================================================================

  /**
   * Simple fuzzy scoring heuristic.
   *
   * Bonus rules (inspired by nucleo-matcher):
   *  - +5  for each consecutive match.
   *  - +3  if the match starts the string.
   *  - +2  if the match follows a word boundary (hyphen, underscore, slash).
   *  - +1  for any other match position.
   *
   * The score is normalised by pattern length so longer queries don't
   * automatically outrank shorter ones.
   *
   * @param {string} pattern — lower-case query
   * @param {string} target — lower-case candidate
   * @returns {number} — score ≥ 0; higher is better
   */
  function fuzzyScore (pattern, target) {
    if (!pattern) return 1;
    let score = 0;
    let tIdx = 0;
    let prevMatch = false;
    for (let pIdx = 0; pIdx < pattern.length; pIdx++) {
      const pc = pattern.charCodeAt(pIdx);
      // Advance target until we find the pattern character.
      while (tIdx < target.length && target.charCodeAt(tIdx) !== pc) {
        tIdx++;
        prevMatch = false;
      }
      if (tIdx >= target.length) return 0; // mismatch — discard
      // Bonus for start-of-string.
      if (tIdx === 0) score += 3;
      // Bonus for word boundary: first char or preceded by [-_/].
      else {
        const prev = target.charCodeAt(tIdx - 1);
        if (prev === 45 || prev === 95 || prev === 47) score += 2;
        else if (!prevMatch) score += 1;
      }
      // Consecutive-match bonus.
      if (prevMatch) score += 5;
      prevMatch = true;
      tIdx++;
    }
    return score / pattern.length;
  }

  /**
   * Perform a fuzzy search across packages and update state.results.
   * Searches both `name` and `attribute` fields; the higher score wins.
   */
  function runSearch () {
    const raw = state.query.trim();
    if (!raw) {
      state.results = state.packages.slice(0, MAX_RESULTS);
      state.selectedIdx = 0;
      renderResults();
      return;
    }
    const pat = raw.toLowerCase();
    const scored = [];
    for (const pkg of state.packages) {
      const name = (pkg.name || '').toLowerCase();
      const attr = (pkg.attribute || '').toLowerCase();
      const nameScore = fuzzyScore(pat, name);
      const attrScore = fuzzyScore(pat, attr);
      const best = Math.max(nameScore, attrScore);
      if (best > 0) {
        scored.push({ pkg, score: best });
      }
    }
    // Sort descending by score, tie-break by name.
    scored.sort((a, b) => {
      if (b.score !== a.score) return b.score - a.score;
      return (a.pkg.name || '').localeCompare(b.pkg.name || '');
    });
    state.results = scored.slice(0, MAX_RESULTS).map(s => s.pkg);
    state.selectedIdx = 0;
    renderResults();
  }

  /**
   * Debounced wrapper so rapid keystrokes don't thrash the DOM.
   */
  function debouncedSearch () {
    clearTimeout(state.debounceTimer);
    state.debounceTimer = setTimeout(runSearch, SEARCH_DEBOUNCE_MS);
  }

  // =====================================================================
  //  RENDERING — RESULTS LIST
  // =====================================================================

  /**
   * Build the results list DOM from state.results and state.selectedIdx.
   * Uses DocumentFragment for a single reflow.
   */
  function renderResults () {
    const frag = document.createDocumentFragment();
    const queryLower = state.query.toLowerCase();

    for (let i = 0; i < state.results.length; i++) {
      const pkg = state.results[i];
      const li = document.createElement('li');
      li.className = 'results-list__item' + (i === state.selectedIdx ? ' results-list__item--selected' : '');
      li.setAttribute('role', 'option');
      li.setAttribute('aria-selected', i === state.selectedIdx ? 'true' : 'false');
      li.dataset.idx = String(i);

      const nameHtml = highlightMatches(pkg.name || '', queryLower);
      const attrHtml = highlightMatches(pkg.attribute || '', queryLower);

      li.innerHTML = `
        <div class="results-list__name">${nameHtml}</div>
        <div class="results-list__attr">${attrHtml}</div>
        ${pkg.synopsis ? `<div class="results-list__synopsis">${escapeHtml(pkg.synopsis)}</div>` : ''}
      `;

      // Flake badge (e.g. nixpkgs / home-manager)
      if (pkg.flake) {
        const flakeBadge = document.createElement('span');
        flakeBadge.className = 'results-list__flake';
        flakeBadge.textContent = pkg.flake;
        li.querySelector('.results-list__name').appendChild(flakeBadge);
      }

      frag.appendChild(li);
    }

    els.resultsList.innerHTML = '';
    els.resultsList.appendChild(frag);
    els.resultsCount.textContent = `${state.results.length} result${state.results.length === 1 ? '' : 's'}`;

    // Keep the selected item in view.
    scrollSelectedIntoView();
  }

  /**
   * Wrap matching characters in <mark> for visual feedback.
   * Case-insensitive but preserves original casing in output.
   */
  function highlightMatches (text, queryLower) {
    if (!queryLower) return escapeHtml(text);
    const textLower = text.toLowerCase();
    let out = '';
    let q = 0;
    for (let i = 0; i < text.length; i++) {
      if (q < queryLower.length && textLower.charCodeAt(i) === queryLower.charCodeAt(q)) {
        out += `<mark>${escapeHtml(text[i])}</mark>`;
        q++;
      } else {
        out += escapeHtml(text[i]);
      }
    }
    return out;
  }

  function escapeHtml (str) {
    if (str == null) return '';
    return String(str)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;');
  }

  function scrollSelectedIntoView () {
    const sel = els.resultsList.querySelector('.results-list__item--selected');
    if (sel) sel.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  }

  // =====================================================================
  //  DETAIL PANEL
  // =====================================================================

  /**
   * Open a package in the detail sidebar and trigger a graph projection.
   * @param {Object} pkg
   */
  function selectPackage (pkg) {
    state.activePkg = pkg;
    state.detailOpen = true;
    els.detailEmpty.hidden = true;
    els.detailContent.hidden = false;
    els.panelDetail.classList.add('is-open');

    els.detailName.textContent = pkg.name || 'unknown';
    renderDetailBadges(pkg);
    renderDetailTab();
    refreshGraph();
  }

  function renderDetailBadges (pkg) {
    const badges = [];
    if (pkg.version) badges.push(`<span class="badge badge--version">${escapeHtml(pkg.version)}</span>`);
    if (pkg.installed) badges.push(`<span class="badge badge--installed">❄ installed</span>`);
    if (pkg.flake) badges.push(`<span class="badge badge--flake">${escapeHtml(pkg.flake)}</span>`);
    if (pkg.licenses && pkg.licenses.length) {
      badges.push(`<span class="badge badge--license">${escapeHtml(pkg.licenses[0])}</span>`);
    }
    els.detailBadges.innerHTML = badges.join('');
  }

  /**
   * Render the active detail tab content.
   */
  function renderDetailTab () {
    const pkg = state.activePkg;
    if (!pkg) return;
    const tab = state.detailTab;
    const body = els.detailBody;
    body.innerHTML = '';

    switch (tab) {
      case 'info':
        body.innerHTML = renderInfoTab(pkg);
        break;
      case 'deps':
        body.innerHTML = renderDepsTab(pkg);
        break;
      case 'reverse':
        body.innerHTML = renderReverseTab(pkg);
        break;
      case 'flake':
        body.innerHTML = renderFlakeTab(pkg);
        break;
    }
  }

  function renderInfoTab (pkg) {
    const homepage = pkg.homepage ? `<a href="${escapeHtml(pkg.homepage)}" target="_blank" rel="noopener">${escapeHtml(pkg.homepage)}</a>` : '—';
    const store = pkg.store_path ? escapeHtml(pkg.store_path) : '—';
    const desc = pkg.description ? escapeHtml(pkg.description) : (pkg.synopsis ? escapeHtml(pkg.synopsis) : 'No description available.');
    return `
      <h3>Description</h3>
      <p>${desc}</p>
      <h3>Metadata</h3>
      <dl>
        <dt>Attribute</dt><dd>${escapeHtml(pkg.attribute || '—')}</dd>
        <dt>Nix file</dt><dd>${escapeHtml(pkg.file ? pkg.file[0] : '—')} :${pkg.file ? pkg.file[1] : ''}</dd>
        <dt>Homepage</dt><dd>${homepage}</dd>
        <dt>Store path</dt><dd>${store}</dd>
      </dl>
    `;
  }

  /**
   * Render the dependency list with kind badges.
   * We resolve numeric IDs back to names via state.packages.
   */
  function renderDepsTab (pkg) {
    let html = `<h3>Dependencies</h3><ul>`;
    const add = (ids, kind) => {
      for (const id of ids) {
        const dep = state.packages[id];
        if (!dep) continue;
        html += `<li>${escapeHtml(dep.name)}<span class="dep-kind dep-kind--${kind}">${kind}</span></li>`;
      }
    };
    add(pkg.inputs || [], 'input');
    add(pkg.propagated_inputs || [], 'propagated');
    add(pkg.native_inputs || [], 'native');
    if ((pkg.inputs || []).length + (pkg.propagated_inputs || []).length + (pkg.native_inputs || []).length === 0) {
      html += `<li style="border:none;color:var(--text-dim)">No declared dependencies.</li>`;
    }
    html += `</ul>`;
    return html;
  }

  function renderReverseTab (pkg) {
    // Reverse deps: packages that list this one in their inputs.
    const rev = [];
    for (const p of state.packages) {
      const all = [
        ...(p.inputs || []),
        ...(p.propagated_inputs || []),
        ...(p.native_inputs || []),
      ];
      if (all.includes(pkg.id)) rev.push(p);
    }
    let html = `<h3>Reverse Dependencies (${rev.length})</h3><ul>`;
    for (const p of rev) {
      html += `<li>${escapeHtml(p.name)}<span class="dep-kind">${escapeHtml(p.attribute || '')}</span></li>`;
    }
    if (!rev.length) html += `<li style="border:none;color:var(--text-dim)">No known reverse dependencies.</li>`;
    html += `</ul>`;
    return html;
  }

  function renderFlakeTab (pkg) {
    const flake = pkg.flake || 'nixpkgs';
    return `
      <h3>Flake Input</h3>
      <p>This package originates from the <strong>${escapeHtml(flake)}</strong> flake.</p>
      <h3>Lock Info</h3>
      <dl>
        <dt>Flake</dt><dd>${escapeHtml(flake)}</dd>
        <dt>Channel</dt><dd>${escapeHtml(state.index?.header?.channel || '—')}</dd>
        <dt>nixpkgs commit</dt><dd>${escapeHtml(state.index?.header?.nixpkgs_commit || '—')}</dd>
      </dl>
    `;
  }

  // =====================================================================
  //  THEME SWITCHING
  // =====================================================================

  function cycleTheme () {
    state.themeIdx = (state.themeIdx + 1) % THEMES.length;
    applyTheme();
  }

  function applyTheme () {
    document.body.setAttribute('data-theme', THEMES[state.themeIdx]);
  }

  // =====================================================================
  //  GRAPH BRIDGE
  // =====================================================================

  /**
   * Build a projection (nodes + edges) from the currently selected package
   * and pass it to the Graph engine (defined in graph.js).
   *
   * Projection format mirrors the Rust `graph::Projection` struct:
   *   { nodes: [id, id, ...], edges: [[from, to], ...], depth_of: [...] }
   */
  function refreshGraph () {
    if (!window.NixGraph) return;
    const pkg = state.activePkg;
    if (!pkg) {
      window.NixGraph.clear();
      els.graphStatus.textContent = '';
      return;
    }

    const { nodes, edges, depth_of } = buildProjection(pkg, state.graphDir, state.graphDepth);

    // Map numeric IDs to display labels.
    const labels = nodes.map(id => {
      const p = state.packages[id];
      return p ? p.name : String(id);
    });

    // Flake colour coding for nodes.
    const nodeColors = nodes.map(id => {
      const p = state.packages[id];
      if (!p) return null;
      if (p.flake === 'home-manager') return 'var(--accent-peach)';
      if (p.installed) return 'var(--accent-teal)';
      return null; // default
    });

    window.NixGraph.setGraph({ nodes, edges, labels, depth_of, nodeColors });
    els.graphStatus.textContent = `${nodes.length} node${nodes.length === 1 ? '' : 's'} · ${edges.length} edge${edges.length === 1 ? '' : 's'}`;
  }

  /**
   * BFS projection of dependencies or reverse-dependencies.
   * @returns {Object} { nodes: number[], edges: number[][], depth_of: number[] }
   */
  function buildProjection (rootPkg, dir, maxDepth) {
    const rootId = rootPkg.id;
    const seen = new Set([rootId]);
    const queue = [[rootId, 0]]; // [id, depth]
    const nodes = [rootId];
    const depthOf = [0];
    const edges = [];

    while (queue.length) {
      const [id, depth] = queue.shift();
      if (depth >= maxDepth) continue;
      const pkg = state.packages[id];
      if (!pkg) continue;

      const getTargets = () => {
        if (dir === 'deps') {
          return [
            ...(pkg.inputs || []),
            ...(pkg.propagated_inputs || []),
            ...(pkg.native_inputs || []),
          ];
        } else {
          // Reverse: scan all packages for references to `id`.
          const rev = [];
          for (const p of state.packages) {
            const all = [
              ...(p.inputs || []),
              ...(p.propagated_inputs || []),
              ...(p.native_inputs || []),
            ];
            if (all.includes(id)) rev.push(p.id);
          }
          return rev;
        }
      };

      for (const childId of getTargets()) {
        if (!seen.has(childId)) {
          seen.add(childId);
          nodes.push(childId);
          depthOf.push(depth + 1);
          queue.push([childId, depth + 1]);
        }
        // Add edge from current node to child.
        const fromIdx = nodes.indexOf(id);
        const toIdx = nodes.indexOf(childId);
        if (fromIdx >= 0 && toIdx >= 0) {
          edges.push([fromIdx, toIdx]);
        }
      }
    }

    return { nodes, edges, depth_of: depthOf };
  }

  // =====================================================================
  //  EVENT HANDLERS
  // =====================================================================

  function onSearchInput () {
    state.query = els.searchInput.value;
    state.suggestIdx = -1;
    debouncedSearch();
    updateAutocomplete();
  }

  /**
   * Populate the autocomplete dropdown with top fuzzy hits.
   */
  function updateAutocomplete () {
    const raw = state.query.trim();
    if (!raw || raw.length < 2) {
      els.searchSuggest.hidden = true;
      return;
    }
    const pat = raw.toLowerCase();
    const hits = [];
    for (const pkg of state.packages) {
      const name = (pkg.name || '').toLowerCase();
      const attr = (pkg.attribute || '').toLowerCase();
      const s = Math.max(fuzzyScore(pat, name), fuzzyScore(pat, attr));
      if (s > 0) hits.push({ pkg, score: s });
    }
    hits.sort((a, b) => b.score - a.score);
    const top = hits.slice(0, MAX_SUGGESTIONS);

    if (!top.length) {
      els.searchSuggest.hidden = true;
      return;
    }

    els.searchSuggest.innerHTML = '';
    for (const h of top) {
      const li = document.createElement('li');
      li.className = 'search-suggest__item';
      li.setAttribute('role', 'option');
      li.innerHTML = highlightMatches(h.pkg.name || '', pat) +
        ` <span style="color:var(--text-dim);font-size:11px">${escapeHtml(h.pkg.attribute || '')}</span>`;
      li.addEventListener('mousedown', (e) => {
        e.preventDefault(); // prevent blur before click
        state.query = h.pkg.name || '';
        els.searchInput.value = state.query;
        runSearch();
        selectPackage(h.pkg);
        els.searchSuggest.hidden = true;
      });
      els.searchSuggest.appendChild(li);
    }
    els.searchSuggest.hidden = false;
  }

  function onResultsClick (e) {
    const item = e.target.closest('.results-list__item');
    if (!item) return;
    const idx = parseInt(item.dataset.idx, 10);
    if (Number.isNaN(idx)) return;
    state.selectedIdx = idx;
    renderResults();
    selectPackage(state.results[idx]);
  }

  function onKeyDown (e) {
    // Help modal takes priority.
    if (state.helpOpen) {
      if (e.key === 'Escape' || e.key === '?') {
        e.preventDefault();
        toggleHelp(false);
      }
      return;
    }

    // Global shortcuts (only when not typing in an input).
    const tag = document.activeElement?.tagName;
    const isTyping = tag === 'INPUT' || tag === 'TEXTAREA';

    if (!isTyping) {
      switch (e.key) {
        case '/':
        case 's':
          e.preventDefault();
          els.searchInput.focus();
          return;
        case 't':
          e.preventDefault();
          cycleTheme();
          return;
        case '?':
          e.preventDefault();
          toggleHelp(true);
          return;
        case 'r':
          e.preventDefault();
          if (window.NixGraph) window.NixGraph.resetView();
          return;
        case 'd':
          e.preventDefault();
          setGraphDir('deps');
          return;
        case 'D':
          e.preventDefault();
          setGraphDir('reverse');
          return;
        case '+':
        case '=':
          e.preventDefault();
          changeDepth(1);
          return;
        case '-':
        case '_':
          e.preventDefault();
          changeDepth(-1);
          return;
        case 'ArrowDown':
          e.preventDefault();
          navResults(1);
          return;
        case 'ArrowUp':
          e.preventDefault();
          navResults(-1);
          return;
        case 'Enter':
          e.preventDefault();
          if (state.results[state.selectedIdx]) {
            selectPackage(state.results[state.selectedIdx]);
          }
          return;
        case 'Escape':
          // Close detail drawer on mobile.
          state.detailOpen = false;
          els.panelDetail.classList.remove('is-open');
          return;
      }
    } else if (isTyping) {
      // Input-specific shortcuts.
      if (e.key === 'Escape') {
        e.preventDefault();
        els.searchInput.blur();
        els.searchSuggest.hidden = true;
        return;
      }
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        els.searchSuggest.hidden = true;
        navResults(1);
        return;
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault();
        els.searchSuggest.hidden = true;
        navResults(-1);
        return;
      }
      if (e.key === 'Enter') {
        els.searchSuggest.hidden = true;
        if (state.results[state.selectedIdx]) {
          selectPackage(state.results[state.selectedIdx]);
          els.searchInput.blur();
        }
        return;
      }
    }
  }

  function navResults (delta) {
    const len = state.results.length;
    if (!len) return;
    state.selectedIdx = (state.selectedIdx + delta + len) % len;
    renderResults();
  }

  function setGraphDir (dir) {
    state.graphDir = dir;
    els.btnGraphDeps.classList.toggle('btn-segment--active', dir === 'deps');
    els.btnGraphRev.classList.toggle('btn-segment--active', dir === 'reverse');
    refreshGraph();
  }

  function changeDepth (delta) {
    const next = Math.max(1, Math.min(8, state.graphDepth + delta));
    state.graphDepth = next;
    els.graphDepth.value = String(next);
    els.graphDepthVal.textContent = String(next);
    refreshGraph();
  }

  function onDetailTabClick (e) {
    const btn = e.target.closest('.detail-tab');
    if (!btn) return;
    state.detailTab = btn.dataset.tab;
    els.detailTabs.forEach(t => {
      t.classList.toggle('detail-tab--active', t.dataset.tab === state.detailTab);
      t.setAttribute('aria-selected', t.dataset.tab === state.detailTab ? 'true' : 'false');
    });
    renderDetailTab();
  }

  function toggleHelp (show) {
    state.helpOpen = show;
    els.helpModal.hidden = !show;
    if (show) els.btnHelpClose.focus();
  }

  // =====================================================================
  //  TOAST
  // =====================================================================

  let toastTimer = null;
  function showToast (msg, type = 'info') {
    els.toast.textContent = msg;
    els.toast.className = 'toast is-visible';
    if (type === 'error') els.toast.style.borderColor = 'var(--accent-red)';
    else els.toast.style.borderColor = '';
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => {
      els.toast.className = 'toast is-hidden';
    }, 3000);
  }

  // =====================================================================
  //  GRAPH TOOLBAR WIRING
  // =====================================================================

  els.btnGraphDeps.addEventListener('click', () => setGraphDir('deps'));
  els.btnGraphRev.addEventListener('click', () => setGraphDir('reverse'));
  els.btnGraphReset.addEventListener('click', () => {
    if (window.NixGraph) window.NixGraph.resetView();
  });
  els.graphDepth.addEventListener('input', (e) => {
    state.graphDepth = parseInt(e.target.value, 10);
    els.graphDepthVal.textContent = String(state.graphDepth);
    refreshGraph();
  });

  // =====================================================================
  //  INITIALISATION
  // =====================================================================

  function init () {
    // Wire event listeners.
    els.searchInput.addEventListener('input', onSearchInput);
    els.searchInput.addEventListener('focus', updateAutocomplete);
    els.searchInput.addEventListener('blur', () => {
      // Delay hiding so mousedown on suggestions can fire first.
      setTimeout(() => { els.searchSuggest.hidden = true; }, 150);
    });
    els.resultsList.addEventListener('click', onResultsClick);
    document.addEventListener('keydown', onKeyDown);
    els.btnTheme.addEventListener('click', cycleTheme);
    els.btnHelp.addEventListener('click', () => toggleHelp(true));
    els.btnHelpClose.addEventListener('click', () => toggleHelp(false));
    els.detailTabs.forEach(t => t.addEventListener('click', onDetailTabClick));

    // Prevent default form submission if inside a form.
    els.searchInput.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') e.preventDefault();
    });

    // Load index (async).
    loadIndex();

    // Apply default theme.
    applyTheme();

    // Focus search by default.
    els.searchInput.focus();
  }

  // Kick off once DOM is ready.
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
