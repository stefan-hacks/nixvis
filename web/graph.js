/**
 * nixvis — Force-Directed Graph Engine (graph.js)
 * =================================================
 * A self-contained HTML5 Canvas renderer for dependency graphs.
 *
 * Algorithm
 * ---------
 * 1. Verlet integration with a configurable time-step.
 * 2. Repulsion: Coulomb-like force between every pair of nodes.
 *    (All-pairs O(n²) — acceptable because the Rust backend caps projections
 *    at ~1 200 nodes.)
 * 3. Attraction: Hookean spring along each edge with a target rest length.
 * 4. Damping: velocity is multiplied each step to prevent unbounded energy.
 * 5. Centering: after simulation, the centroid is shifted to the canvas centre.
 *
 * Interactivity
 * -------------
 * - Pan: click-drag on empty canvas area.
 * - Zoom: mouse wheel (or pinch on touch devices).
 * - Hover: highlights a node and displays its label in a floating tooltip.
 * - Click: sets the node as "focused" (accent colour) and broadcasts to app.js.
 *
 * Rendering pipeline (per frame)
 * ------------------------------
 *   clear → transform (pan/zoom) → draw edges → draw nodes → draw labels
 *
 * The transform is stored as an affine matrix { scale, tx, ty } so panning
 * and zooming are mathematically correct and independent of the simulation
 * coordinates.
 *
 * Nix-specific visual touches
 * ---------------------------
 * - Nodes are drawn as snowflakes (❄) instead of plain circles when zoomed in.
 * - Accent colours from CSS custom properties are read at runtime so theming
 *   changes in app.js automatically propagate here.
 * - Depth is encoded as opacity (deeper nodes are more translucent).
 */

(function () {
  'use strict';

  // =====================================================================
  //  CONSTANTS
  // =====================================================================

  /** Physics simulation parameters.  Tuned for readability on dark backgrounds. */
  const PHYS = {
    /** Coulomb repulsion constant. */
    REPULSION: 1200.0,
    /** Hooke spring constant. */
    SPRING_K: 0.015,
    /** Velocity damping factor (0–1). */
    DAMPING: 0.92,
    /** Integration time-step. */
    DT: 0.6,
    /** Number of simulation steps to run after each graph change. */
    STEPS: 350,
    /** Target rest length for springs (pixels in simulation space). */
    REST_LENGTH: 60.0,
  };

  /** Canvas rendering parameters. */
  const RENDER = {
    /** Base node radius (simulation units). */
    NODE_RADIUS: 6,
    /** Line width for edges. */
    EDGE_WIDTH: 1.5,
    /** Maximum opacity for deepest nodes. */
    MIN_OPACITY: 0.35,
    /** Font size for labels. */
    LABEL_SIZE: 12,
    /** Zoom sensitivity. */
    ZOOM_SENSITIVITY: 0.0015,
    /** Minimum zoom level. */
    MIN_ZOOM: 0.2,
    /** Maximum zoom level. */
    MAX_ZOOM: 4.0,
  };

  // =====================================================================
  //  STATE
  // =====================================================================

  const canvas = document.getElementById('graph-canvas');
  const ctx = canvas.getContext('2d');
  const statusEl = document.getElementById('graph-status');
  const hintEl = document.getElementById('graph-hint');

  /**
   * Current graph data.  Set via NixGraph.setGraph() from app.js.
   * @type {Object|null}
   */
  let graphData = null;

  /**
   * Computed simulation positions: { x, y } for each node.
   * These are in simulation space (origin at centroid).
   * @type {Array<{x:number, y:number}>}
   */
  let simPos = [];

  /**
   * Computed simulation velocities.
   * @type {Array<{x:number, y:number}>}
   */
  let simVel = [];

  /**
   * Viewport transform: pan offset + zoom scale.
   * The affine mapping from simulation → screen is:
   *   screenX = (simX * scale) + tx
   *   screenY = (simY * scale) + ty
   * @type {{scale:number, tx:number, ty:number}}
   */
  let transform = { scale: 1, tx: 0, ty: 0 };

  /**
   * Currently hovered node index (-1 = none).
   * @type {number}
   */
  let hoverIdx = -1;

  /**
   * Currently focused (clicked) node index (-1 = none).
   * @type {number}
   */
  let focusIdx = -1;

  /**
   * Drag state for panning.
   * @type {{active:boolean, lastX:number, lastY:number}}
   */
  let drag = { active: false, lastX: 0, lastY: 0 };

  /**
   * Whether the physics simulation has settled enough to stop requesting
   * animation frames.
   * @type {boolean}
   */
  let settled = true;

  /**
   * rAF handle so we can cancel redundant loops.
   * @type {number|null}
   */
  let rafHandle = null;

  /**
   * Resize observer for responsive canvas sizing.
   * @type {ResizeObserver|null}
   */
  let resizeObs = null;

  // =====================================================================
  //  COLOUR HELPERS
  // =====================================================================

  /**
   * Read a CSS custom property from the document root (or body).
   * Falls back to a safe default if the property is missing.
   */
  function cssVar (name, fallback) {
    const val = getComputedStyle(document.body).getPropertyValue(name).trim();
    return val || fallback;
  }

  /**
   * Parse an rgb/rgba or hex colour string into an {r,g,b} object.
   * Supports #rgb, #rrggbb, and rgb(r,g,b) formats.
   */
  function parseColor (str) {
    str = str.trim();
    // Hex short.
    if (str[0] === '#' && str.length === 4) {
      return {
        r: parseInt(str[1] + str[1], 16),
        g: parseInt(str[2] + str[2], 16),
        b: parseInt(str[3] + str[3], 16),
      };
    }
    // Hex long.
    if (str[0] === '#' && str.length === 7) {
      return {
        r: parseInt(str.slice(1, 3), 16),
        g: parseInt(str.slice(3, 5), 16),
        b: parseInt(str.slice(5, 7), 16),
      };
    }
    // rgb()
    const m = str.match(/rgba?\((\d+),\s*(\d+),\s*(\d+)\)/);
    if (m) {
      return { r: +m[1], g: +m[2], b: +m[3] };
    }
    return { r: 137, g: 180, b: 250 }; // fallback blue
  }

  // =====================================================================
  //  SIMULATION
  // =====================================================================

  /**
   * Initialise positions on a circle so the graph starts from a known,
   * unbiased state.  This avoids different runs producing wildly different
   * layouts for the same graph.
   */
  function initPositions (n) {
    simPos = [];
    simVel = [];
    const angleStep = (2 * Math.PI) / Math.max(n, 1);
    for (let i = 0; i < n; i++) {
      const r = 100 + (i % 5) * 40;
      const a = i * angleStep;
      simPos.push({ x: r * Math.cos(a), y: r * Math.sin(a) });
      simVel.push({ x: 0, y: 0 });
    }
  }

  /**
   * Run the force-directed simulation for PHYS.STEPS iterations.
   * This is invoked once per graph change (not every frame), so it can
   * be relatively heavy.
   */
  function runSimulation (nodes, edges) {
    const n = nodes.length;
    if (n === 0) return;
    initPositions(n);

    for (let step = 0; step < PHYS.STEPS; step++) {
      // Forces accumulator.
      const fx = new Float64Array(n);
      const fy = new Float64Array(n);

      // Repulsion (all-pairs O(n²)).
      for (let i = 0; i < n; i++) {
        for (let j = i + 1; j < n; j++) {
          const dx = simPos[i].x - simPos[j].x;
          const dy = simPos[i].y - simPos[j].y;
          const distSq = dx * dx + dy * dy + 0.01;
          const dist = Math.sqrt(distSq);
          const force = PHYS.REPULSION / distSq;
          const fX = (force * dx) / dist;
          const fY = (force * dy) / dist;
          fx[i] += fX;
          fy[i] += fY;
          fx[j] -= fX;
          fy[j] -= fY;
        }
      }

      // Spring attraction along edges.
      for (let e = 0; e < edges.length; e++) {
        const [ia, ib] = edges[e];
        const i = ia;
        const j = ib;
        const dx = simPos[j].x - simPos[i].x;
        const dy = simPos[j].y - simPos[i].y;
        const dist = Math.sqrt(dx * dx + dy * dy) || 0.01;
        const force = PHYS.SPRING_K * (dist - PHYS.REST_LENGTH);
        const fX = (force * dx) / dist;
        const fY = (force * dy) / dist;
        fx[i] += fX;
        fy[i] += fY;
        fx[j] -= fX;
        fy[j] -= fY;
      }

      // Integrate (Verlet with damping).
      for (let i = 0; i < n; i++) {
        simVel[i].x = (simVel[i].x + fx[i] * PHYS.DT) * PHYS.DAMPING;
        simVel[i].y = (simVel[i].y + fy[i] * PHYS.DT) * PHYS.DAMPING;
        simPos[i].x += simVel[i].x * PHYS.DT;
        simPos[i].y += simVel[i].y * PHYS.DT;
      }
    }

    // Center around origin.
    let cx = 0, cy = 0;
    for (let i = 0; i < n; i++) {
      cx += simPos[i].x;
      cy += simPos[i].y;
    }
    cx /= n;
    cy /= n;
    for (let i = 0; i < n; i++) {
      simPos[i].x -= cx;
      simPos[i].y -= cy;
    }
  }

  // =====================================================================
  //  RENDERING
  // =====================================================================

  /**
   * Resize the canvas to match its CSS size and handle device-pixel-ratio
   * for crisp rendering on Hi-DPI displays.
   */
  function resizeCanvas () {
    const rect = canvas.parentElement.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.floor(rect.width * dpr);
    canvas.height = Math.floor(rect.height * dpr);
    canvas.style.width = rect.width + 'px';
    canvas.style.height = rect.height + 'px';
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0); // reset to CSS pixels
    resetView();
    requestRender();
  }

  /**
   * Fit the graph into the viewport with a comfortable margin.
   */
  function resetView () {
    if (!simPos.length) {
      transform = { scale: 1, tx: canvas.clientWidth / 2, ty: canvas.clientHeight / 2 };
      return;
    }
    const xs = simPos.map(p => p.x);
    const ys = simPos.map(p => p.y);
    const minX = Math.min(...xs);
    const maxX = Math.max(...xs);
    const minY = Math.min(...ys);
    const maxY = Math.max(...ys);
    const graphW = Math.max(maxX - minX, 1);
    const graphH = Math.max(maxY - minY, 1);
    const margin = 60;
    const availW = canvas.clientWidth - margin * 2;
    const availH = canvas.clientHeight - margin * 2;
    const scale = Math.min(availW / graphW, availH / graphH);
    transform = {
      scale: scale,
      tx: (canvas.clientWidth - graphW * scale) / 2 - minX * scale,
      ty: (canvas.clientHeight - graphH * scale) / 2 - minY * scale,
    };
    requestRender();
  }

  /**
   * Convert simulation coordinates to screen coordinates.
   */
  function toScreen (sx, sy) {
    return {
      x: sx * transform.scale + transform.tx,
      y: sy * transform.scale + transform.ty,
    };
  }

  /**
   * Convert screen coordinates to simulation coordinates.
   */
  function toSim (px, py) {
    return {
      x: (px - transform.tx) / transform.scale,
      y: (py - transform.ty) / transform.scale,
    };
  }

  /**
   * Main render function.
   */
  function render () {
    const w = canvas.clientWidth;
    const h = canvas.clientHeight;
    ctx.clearRect(0, 0, w, h);

    if (!graphData || !simPos.length) {
      drawEmptyState(w, h);
      return;
    }

    const { edges, labels, depth_of: depthOf, nodeColors } = graphData;
    const n = simPos.length;

    // Read colours from CSS so they track the active theme.
    const edgeColor = parseColor(cssVar('--graph-edge', '#45475a'));
    const nodeColor = parseColor(cssVar('--graph-node', '#89b4fa'));
    const focusColor = parseColor(cssVar('--graph-focus', '#f9e2af'));
    const textColor = parseColor(cssVar('--text', '#cdd6f4'));

    // Draw edges first (so nodes sit on top).
    ctx.lineWidth = RENDER.EDGE_WIDTH * Math.max(0.5, transform.scale);
    for (let e = 0; e < edges.length; e++) {
      const [a, b] = edges[e];
      const pa = toScreen(simPos[a].x, simPos[a].y);
      const pb = toScreen(simPos[b].x, simPos[b].y);

      // Edge opacity fades with deeper depth.
      const maxDepth = Math.max(1, ...depthOf);
      const depthAlpha = 1 - (depthOf[b] / (maxDepth + 1)) * 0.6;
      ctx.strokeStyle = `rgba(${edgeColor.r},${edgeColor.g},${edgeColor.b},${Math.max(0.15, depthAlpha)})`;
      ctx.beginPath();
      ctx.moveTo(pa.x, pa.y);
      ctx.lineTo(pb.x, pb.y);
      ctx.stroke();
    }

    // Draw nodes.
    const baseRadius = RENDER.NODE_RADIUS * Math.max(0.6, transform.scale);
    for (let i = 0; i < n; i++) {
      const p = toScreen(simPos[i].x, simPos[i].y);
      const isFocus = i === focusIdx;
      const isHover = i === hoverIdx;
      const depth = depthOf[i] || 0;
      const opacity = Math.max(RENDER.MIN_OPACITY, 1 - depth * 0.12);

      // Determine fill colour.
      let fill = nodeColor;
      if (isFocus) fill = focusColor;
      else if (nodeColors && nodeColors[i]) {
        fill = parseColor(nodeColors[i]);
      }

      // Glow for focus / hover.
      if (isFocus || isHover) {
        ctx.save();
        ctx.shadowColor = `rgba(${fill.r},${fill.g},${fill.b},0.45)`;
        ctx.shadowBlur = 16;
      }

      ctx.globalAlpha = opacity;
      ctx.fillStyle = `rgb(${fill.r},${fill.g},${fill.b})`;

      // Draw node shape: circle, larger if focused.
      const r = isFocus ? baseRadius * 1.6 : (isHover ? baseRadius * 1.3 : baseRadius);
      ctx.beginPath();
      ctx.arc(p.x, p.y, r, 0, Math.PI * 2);
      ctx.fill();

      // If zoomed in enough, draw a snowflake emoji instead of a plain circle
      // for root nodes (depth === 0).
      if (transform.scale > 1.4 && depth === 0) {
        ctx.font = `${Math.floor(r * 2.5)}px serif`;
        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        ctx.fillStyle = `rgb(${fill.r},${fill.g},${fill.b})`;
        ctx.fillText('❄', p.x, p.y);
      }

      if (isFocus || isHover) ctx.restore();
      ctx.globalAlpha = 1.0;
    }

    // Draw labels for hovered / focused nodes, and root node.
    ctx.font = `${RENDER.LABEL_SIZE}px var(--font-mono), monospace`;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'top';
    for (let i = 0; i < n; i++) {
      if (!(i === hoverIdx || i === focusIdx || (depthOf[i] || 0) === 0)) continue;
      const p = toScreen(simPos[i].x, simPos[i].y);
      const label = (labels && labels[i]) ? labels[i] : String(i);
      ctx.fillStyle = `rgb(${textColor.r},${textColor.g},${textColor.b})`;
      ctx.fillText(label, p.x, p.y + baseRadius + 4);
    }
  }

  function drawEmptyState (w, h) {
    ctx.fillStyle = cssVar('--text-dim', '#6c7086');
    ctx.font = '14px var(--font-ui), sans-serif';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText('Select a package to visualise its Nix dependency graph', w / 2, h / 2);
  }

  /**
   * Schedule a render on the next animation frame, deduplicating requests.
   */
  function requestRender () {
    if (rafHandle !== null) return;
    rafHandle = requestAnimationFrame(() => {
      rafHandle = null;
      render();
    });
  }

  // =====================================================================
  //  INTERACTION HANDLERS
  // =====================================================================

  function onMouseMove (e) {
    const rect = canvas.getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;

    if (drag.active) {
      const dx = mx - drag.lastX;
      const dy = my - drag.lastY;
      transform.tx += dx;
      transform.ty += dy;
      drag.lastX = mx;
      drag.lastY = my;
      requestRender();
      return;
    }

    // Hit-test nodes.
    const sim = toSim(mx, my);
    let bestIdx = -1;
    let bestDist = Infinity;
    for (let i = 0; i < simPos.length; i++) {
      const dx = simPos[i].x - sim.x;
      const dy = simPos[i].y - sim.y;
      const d = Math.sqrt(dx * dx + dy * dy);
      if (d < bestDist) {
        bestDist = d;
        bestIdx = i;
      }
    }
    const hitRadius = 12 / transform.scale; // generous hit area
    const newHover = bestDist <= hitRadius ? bestIdx : -1;
    if (newHover !== hoverIdx) {
      hoverIdx = newHover;
      if (hoverIdx >= 0 && graphData && graphData.labels) {
        hintEl.textContent = graphData.labels[hoverIdx] || '';
        hintEl.hidden = false;
      } else {
        hintEl.hidden = true;
      }
      requestRender();
    }
  }

  function onMouseDown (e) {
    const rect = canvas.getBoundingClientRect();
    drag.active = true;
    drag.lastX = e.clientX - rect.left;
    drag.lastY = e.clientY - rect.top;
    canvas.style.cursor = 'grabbing';

    // Click-to-focus node.
    const mx = drag.lastX;
    const my = drag.lastY;
    const sim = toSim(mx, my);
    for (let i = 0; i < simPos.length; i++) {
      const dx = simPos[i].x - sim.x;
      const dy = simPos[i].y - sim.y;
      if (Math.sqrt(dx * dx + dy * dy) <= 10 / transform.scale) {
        focusIdx = i;
        requestRender();
        break;
      }
    }
  }

  function onMouseUp () {
    drag.active = false;
    canvas.style.cursor = 'grab';
  }

  function onWheel (e) {
    e.preventDefault();
    const rect = canvas.getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;

    // Zoom around cursor.
    const zoomFactor = Math.exp(-e.deltaY * RENDER.ZOOM_SENSITIVITY);
    const newScale = Math.min(RENDER.MAX_ZOOM, Math.max(RENDER.MIN_ZOOM, transform.scale * zoomFactor));

    // Keep the point under the cursor stable.
    const simX = (mx - transform.tx) / transform.scale;
    const simY = (my - transform.ty) / transform.scale;
    transform.tx = mx - simX * newScale;
    transform.ty = my - simY * newScale;
    transform.scale = newScale;

    requestRender();
  }

  // =====================================================================
  //  PUBLIC API (exposed as window.NixGraph)
  // =====================================================================

  /**
   * Set a new graph and kick off the physics simulation.
   * @param {Object} data
   * @param {number[]} data.nodes — package IDs
   * @param {number[][]} data.edges — [fromIdx, toIdx]
   * @param {string[]} [data.labels] — display labels per node
   * @param {number[]} [data.depth_of] — BFS depth per node
   * @param {string[]} [data.nodeColors] — per-node CSS colour overrides
   */
  function setGraph (data) {
    graphData = data;
    focusIdx = 0; // root node
    hoverIdx = -1;
    runSimulation(data.nodes, data.edges);
    resetView();
  }

  /**
   * Clear the canvas and reset internal state.
   */
  function clear () {
    graphData = null;
    simPos = [];
    simVel = [];
    focusIdx = -1;
    hoverIdx = -1;
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    if (statusEl) statusEl.textContent = '';
  }

  // =====================================================================
  //  INITIALISATION
  // =====================================================================

  function init () {
    // Responsive sizing.
    resizeObs = new ResizeObserver(() => resizeCanvas());
    resizeObs.observe(canvas.parentElement);
    resizeCanvas();

    // Events.
    canvas.addEventListener('mousemove', onMouseMove);
    canvas.addEventListener('mousedown', onMouseDown);
    window.addEventListener('mouseup', onMouseUp);
    canvas.addEventListener('wheel', onWheel, { passive: false });

    // Expose API.
    window.NixGraph = { setGraph, clear, resetView };
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
