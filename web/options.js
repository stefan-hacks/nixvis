// Standalone Options Browser for nixvis web SPA.
// Loads into the existing DOM by creating a dedicated options panel.

(function () {
  'use strict';

  const API_NIXOS = '/api/v1/options/nixos';
  const API_HM = '/api/v1/options/hm';

  let currentMode = 'packages';
  let currentQ = '';
  let results = [];
  let selectedIdx = 0;

  function $(sel) { return document.querySelector(sel); }

  // =====================================================================
  //  MODE SWITCHER
  // =====================================================================
  const switcher = $('#mode-switcher');
  if (switcher) {
    switcher.addEventListener('click', (e) => {
      const btn = e.target.closest('.mode-btn');
      if (!btn) return;
      setMode(btn.dataset.mode);
    });

    // Keyboard shortcuts: 1=packages, 2=nixos, 3=hm
    document.addEventListener('keydown', (e) => {
      if (e.target.tagName === 'INPUT') return;
      if (e.key === '1') { e.preventDefault(); setMode('packages'); }
      else if (e.key === '2') { e.preventDefault(); setMode('nixos'); }
      else if (e.key === '3') { e.preventDefault(); setMode('hm'); }
    });
  }

  function setMode (mode) {
    currentMode = mode;
    const btns = switcher.querySelectorAll('.mode-btn');
    btns.forEach((b) => {
      const active = b.dataset.mode === mode;
      b.classList.toggle('is-active', active);
      b.setAttribute('aria-selected', active ? 'true' : 'false');
    });

    const isPkg = mode === 'packages';
    const pkgPanel = $('.panel--search');
    const detailPanel = $('#panel-detail');
    let optPanel = $('#panel-options');

    if (isPkg) {
      pkgPanel.hidden = false;
      if (optPanel) optPanel.hidden = true;
      if (detailPanel) detailPanel.hidden = false;
    } else {
      pkgPanel.hidden = true;
      if (detailPanel) detailPanel.hidden = true;
      if (!optPanel) optPanel = createOptionsPanel();
      optPanel.hidden = false;
      refreshOptions();
    }
  }

  // =====================================================================
  //  OPTIONS PANEL DOM
  // =====================================================================
  function createOptionsPanel () {
    const panel = document.createElement('section');
    panel.id = 'panel-options';
    panel.className = 'panel panel--options';
    panel.setAttribute('aria-label', 'Options browser');
    panel.innerHTML = `
      <div class="options-search-wrap">
        <input id="options-input" class="search-input" type="text"
          placeholder="Search ${currentMode === 'nixos' ? 'NixOS' : 'Home-Manager'} options…"
          autocomplete="off" spellcheck="false">
        <span class="results-meta__count" id="options-count">0 options</span>
      </div>
      <ul id="options-list" class="results-list" role="listbox"></ul>
      <div id="options-detail" class="options-detail" hidden>
        <h3 class="options-detail__name" id="opt-detail-name"></h3>
        <p class="options-detail__type" id="opt-detail-type"></p>
        <div class="options-detail__section">
          <h4>Description</h4>
          <p id="opt-detail-desc"></p>
        </div>
        <div class="options-detail__section">
          <h4>Default</h4>
          <pre id="opt-detail-default"><code></code></pre>
        </div>
      </div>
    `;
    $('.main-grid').appendChild(panel);

    panel.querySelector('#options-input').addEventListener('input', debounce((e) => {
      currentQ = e.target.value.trim();
      refreshOptions();
    }, 200));

    panel.querySelector('#options-list').addEventListener('click', (e) => {
      const item = e.target.closest('.results-list__item');
      if (!item) return;
      selectedIdx = parseInt(item.dataset.idx, 10);
      renderOptionsList();
      showOptionDetail(results[selectedIdx]);
    });

    return panel;
  }

  // =====================================================================
  //  FETCH + RENDER
  // =====================================================================
  async function refreshOptions () {
    const endpoint = currentMode === 'nixos' ? API_NIXOS : API_HM;
    const url = currentQ ? `${endpoint}?q=${encodeURIComponent(currentQ)}` : endpoint;
    try {
      const res = await fetch(url);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = await res.json();
      results = data.items || [];
      selectedIdx = 0;
      renderOptionsList();
      updateCount();
    } catch (err) {
      console.error('options fetch failed:', err);
      results = [];
      renderOptionsList();
    }
  }

  function renderOptionsList () {
    const list = $('#options-list');
    if (!list) return;
    list.innerHTML = results.map((opt, i) => `
      <li class="results-list__item ${i === selectedIdx ? 'is-selected' : ''}"
          data-idx="${i}" role="option" aria-selected="${i === selectedIdx}">
        <div class="result-name">${escapeHtml(opt.name)}</div>
        <div class="result-meta">${escapeHtml(opt.type || '')}</div>
      </li>
    `).join('');
  }

  function showOptionDetail (opt) {
    const detail = $('#options-detail');
    if (!detail || !opt) return;
    detail.hidden = false;
    $('#opt-detail-name').textContent = opt.name;
    $('#opt-detail-type').textContent = opt.type || '—';
    $('#opt-detail-desc').textContent = opt.description || 'No description available.';
    const defEl = $('#opt-detail-default');
    if (defEl) defEl.querySelector('code').textContent = opt.default || '—';
  }

  function updateCount () {
    const el = $('#options-count');
    if (el) el.textContent = `${results.length} option${results.length === 1 ? '' : 's'}`;
  }

  // =====================================================================
  //  UTILITIES
  // =====================================================================
  function escapeHtml (s) {
    if (s == null) return '';
    return String(s)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;');
  }

  function debounce (fn, ms) {
    let t;
    return (...args) => {
      clearTimeout(t);
      t = setTimeout(() => fn(...args), ms);
    };
  }
})();