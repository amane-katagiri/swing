const NODE_RADIUS = 10;
const ARROW_LEN = 8;
const LABEL_MAX = 16;
const LINK_DISTANCE = 130;
const REPULSION = 2600;
const INITIAL_SPREAD_RADIUS = 150;
const SVG_NS = 'http://www.w3.org/2000/svg';

function truncateLabel(label) {
  if (label.length <= LABEL_MAX) return label;
  return label.slice(0, LABEL_MAX - 1) + '…';
}

function clamp(v, lo, hi) {
  return Math.min(hi, Math.max(lo, v));
}

function reducedMotion() {
  return typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
}

function buildArrowMarker(id, fillClass) {
  const marker = document.createElementNS(SVG_NS, 'marker');
  marker.setAttribute('id', id);
  marker.setAttribute('viewBox', '0 0 10 10');
  marker.setAttribute('refX', '0');
  marker.setAttribute('refY', '5');
  marker.setAttribute('markerWidth', String(ARROW_LEN));
  marker.setAttribute('markerHeight', String(ARROW_LEN));
  marker.setAttribute('markerUnits', 'userSpaceOnUse');
  marker.setAttribute('orient', 'auto-start-reverse');
  const path = document.createElementNS(SVG_NS, 'path');
  path.setAttribute('d', 'M 0 0 L 10 5 L 0 10 z');
  path.setAttribute('class', fillClass);
  marker.append(path);
  return marker;
}

function svgNodeSwatch(attrs) {
  const svg = document.createElementNS(SVG_NS, 'svg');
  svg.setAttribute('class', 'swing-legend-swatch');
  svg.setAttribute('viewBox', '0 0 18 18');
  const g = document.createElementNS(SVG_NS, 'g');
  g.setAttribute('class', 'swing-node');
  for (const [k, v] of Object.entries(attrs)) g.dataset[k] = v;
  const circle = document.createElementNS(SVG_NS, 'circle');
  circle.setAttribute('cx', '9');
  circle.setAttribute('cy', '9');
  circle.setAttribute('r', '6');
  g.append(circle);
  svg.append(g);
  return svg;
}

function svgEdgeSwatch(mutual) {
  const svg = document.createElementNS(SVG_NS, 'svg');
  svg.setAttribute('class', 'swing-legend-swatch');
  svg.setAttribute('viewBox', '0 0 28 18');
  const line = document.createElementNS(SVG_NS, 'line');
  line.setAttribute('class', 'swing-edge');
  line.dataset.mutual = String(!!mutual);
  line.setAttribute('x1', '4');
  line.setAttribute('y1', '9');
  line.setAttribute('x2', '24');
  line.setAttribute('y2', '9');
  line.setAttribute('marker-end', mutual ? 'url(#swing-arrow-mutual)' : 'url(#swing-arrow-oneway)');
  if (mutual) line.setAttribute('marker-start', 'url(#swing-arrow-mutual)');
  svg.append(line);
  return svg;
}

function buildLegend(labels) {
  const legend = document.createElement('div');
  legend.className = 'swing-legend';
  const rows = [
    [svgNodeSwatch({ root: 'true' }), labels.root],
    [svgEdgeSwatch(true), labels.mutual],
    [svgEdgeSwatch(false), labels.oneway],
    [svgNodeSwatch({ hasFollowSet: 'false' }), labels.noFollowSet],
  ];
  for (const [swatch, label] of rows) {
    const item = document.createElement('div');
    item.className = 'swing-legend-item';
    const span = document.createElement('span');
    span.textContent = label;
    item.append(swatch, span);
    legend.append(item);
  }
  return legend;
}

const DEFAULT_LABELS = {
  root: 'root',
  mutual: 'mutual',
  oneway: 'one-way (from → to)',
  noFollowSet: 'no follow set',
  fit: 'Fit',
  ariaLabel: 'Webring graph',
  empty: 'No nodes to show for this root and depth.',
};

export function createWebringGraph(container, { nodes, edges, onSelect, selectedPubkey, labels }) {
  const strings = Object.assign({}, DEFAULT_LABELS, labels || {});
  container.replaceChildren();

  if (nodes.length === 0) {
    const empty = document.createElement('p');
    empty.className = 'swing-hint';
    empty.textContent = strings.empty;
    container.append(empty);
    return { destroy() {}, setSelected() {}, fit() {} };
  }

  const wrap = document.createElement('div');
  wrap.className = 'swing-graph-wrap';

  const svg = document.createElementNS(SVG_NS, 'svg');
  svg.setAttribute('class', 'swing-graph');
  svg.setAttribute('role', 'img');
  svg.setAttribute('aria-label', strings.ariaLabel);

  const defs = document.createElementNS(SVG_NS, 'defs');
  defs.append(
    buildArrowMarker('swing-arrow-oneway', 'swing-arrow-oneway-fill'),
    buildArrowMarker('swing-arrow-mutual', 'swing-arrow-mutual-fill'),
  );
  const viewport = document.createElementNS(SVG_NS, 'g');
  const edgeLayer = document.createElementNS(SVG_NS, 'g');
  const nodeLayer = document.createElementNS(SVG_NS, 'g');
  viewport.append(edgeLayer, nodeLayer);
  svg.append(defs, viewport);

  const controls = document.createElement('div');
  controls.className = 'swing-graph-controls';
  const fitBtn = document.createElement('button');
  fitBtn.type = 'button';
  fitBtn.className = 'swing-btn swing-btn-small';
  fitBtn.textContent = strings.fit;
  fitBtn.setAttribute('aria-label', strings.fit);
  fitBtn.title = strings.fit;
  controls.append(fitBtn);

  wrap.append(svg, controls);
  container.append(wrap, buildLegend(strings));

  const single = nodes.length === 1;
  const sim = nodes.map((n, i) => {
    const angle = (i / nodes.length) * Math.PI * 2;
    const radius = single ? 0 : INITIAL_SPREAD_RADIUS;
    return {
      data: n,
      x: Math.cos(angle) * radius,
      y: Math.sin(angle) * radius,
      vx: 0,
      vy: 0,
      fx: null,
      fy: null,
    };
  });
  const byId = new Map(sim.map((s) => [s.data.pubkey, s]));
  const simEdges = edges
    .map((e) => ({ edge: e, a: byId.get(e.from), b: byId.get(e.to) }))
    .filter((e) => e.a && e.b);

  const view = { x: 0, y: 0, zoom: 1 };
  let selected = selectedPubkey || null;
  let userAdjustedView = false;
  const nodeEls = new Map();
  const labelEls = [];
  let baseLabelFontPx = null;

  function updateSelection() {
    for (const [pk, g] of nodeEls) {
      g.dataset.selected = String(pk === selected);
    }
  }

  function buildNodes() {
    nodeLayer.replaceChildren();
    nodeEls.clear();
    labelEls.length = 0;
    for (const s of sim) {
      const g = document.createElementNS(SVG_NS, 'g');
      g.setAttribute('class', 'swing-node');
      g.dataset.root = String(!!s.data.root);
      g.dataset.hasFollowSet = String(!!s.data.has_follow_set);
      g.dataset.depth = String(s.data.depth);
      g.dataset.pubkey = s.data.pubkey;
      g.setAttribute('tabindex', '0');
      g.setAttribute('role', 'button');
      g.setAttribute('aria-label', s.data.label);

      const circle = document.createElementNS(SVG_NS, 'circle');
      circle.setAttribute('r', String(NODE_RADIUS));

      const title = document.createElementNS(SVG_NS, 'title');
      title.textContent = s.data.label;

      const text = document.createElementNS(SVG_NS, 'text');
      text.setAttribute('x', String(NODE_RADIUS + 4));
      text.setAttribute('y', '4');
      text.textContent = truncateLabel(s.data.label);

      g.append(circle, title, text);
      labelEls.push(text);
      g.addEventListener('pointerdown', (ev) => startNodeDrag(ev, s, g));
      g.addEventListener('keydown', (ev) => {
        if (ev.key !== 'Enter' && ev.key !== ' ' && ev.key !== 'Spacebar') return;
        ev.preventDefault();
        selected = s.data.pubkey;
        updateSelection();
        if (onSelect) onSelect(s.data);
      });
      nodeLayer.append(g);
      nodeEls.set(s.data.pubkey, g);
    }
    updateSelection();
  }

  function buildEdges() {
    edgeLayer.replaceChildren();
    for (const e of simEdges) {
      const line = document.createElementNS(SVG_NS, 'line');
      line.setAttribute('class', 'swing-edge');
      line.dataset.mutual = String(!!e.edge.mutual);
      line.setAttribute('marker-end', e.edge.mutual ? 'url(#swing-arrow-mutual)' : 'url(#swing-arrow-oneway)');
      if (e.edge.mutual) line.setAttribute('marker-start', 'url(#swing-arrow-mutual)');
      edgeLayer.append(line);
      e.line = line;
    }
  }

  function applyLabelScale() {
    if (labelEls.length === 0) return;
    if (baseLabelFontPx == null) {
      const parsed = parseFloat(getComputedStyle(labelEls[0]).fontSize);
      baseLabelFontPx = Number.isFinite(parsed) && parsed > 0 ? parsed : 11;
    }
    const effective = baseLabelFontPx / clamp(view.zoom, 0.15, 4);
    const fontSize = `${effective.toFixed(2)}px`;
    for (const text of labelEls) text.style.fontSize = fontSize;
  }

  function render() {
    applyLabelScale();
    for (const s of sim) {
      const g = nodeEls.get(s.data.pubkey);
      if (g) g.setAttribute('transform', `translate(${s.x.toFixed(1)},${s.y.toFixed(1)})`);
    }
    for (const e of simEdges) {
      const dx = e.b.x - e.a.x;
      const dy = e.b.y - e.a.y;
      const dist = Math.sqrt(dx * dx + dy * dy) || 1;
      const ux = dx / dist;
      const uy = dy / dist;
      const trimA = NODE_RADIUS + (e.edge.mutual ? ARROW_LEN : 0);
      const trimB = NODE_RADIUS + ARROW_LEN;
      const maxTrim = Math.max(0, dist - 1);
      const total = trimA + trimB;
      const scale = total > maxTrim ? maxTrim / total : 1;
      const ta = trimA * scale;
      const tb = trimB * scale;
      e.line.setAttribute('x1', (e.a.x + ux * ta).toFixed(1));
      e.line.setAttribute('y1', (e.a.y + uy * ta).toFixed(1));
      e.line.setAttribute('x2', (e.b.x - ux * tb).toFixed(1));
      e.line.setAttribute('y2', (e.b.y - uy * tb).toFixed(1));
    }
    viewport.setAttribute('transform', `translate(${view.x.toFixed(1)},${view.y.toFixed(1)}) scale(${view.zoom.toFixed(3)})`);
  }

  function screenToGraph(clientX, clientY) {
    const rect = svg.getBoundingClientRect();
    const sx = clientX - rect.left;
    const sy = clientY - rect.top;
    return { x: (sx - view.x) / view.zoom, y: (sy - view.y) / view.zoom };
  }

  function fit() {
    const rect = svg.getBoundingClientRect();
    const w = rect.width || 600;
    const h = rect.height || 520;
    let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
    for (const s of sim) {
      minX = Math.min(minX, s.x);
      maxX = Math.max(maxX, s.x);
      minY = Math.min(minY, s.y);
      maxY = Math.max(maxY, s.y);
    }
    const padLeft = clamp(w * 0.06, 24, 50);
    const padRight = clamp(w * 0.2, 60, 190);
    const padY = clamp(h * 0.09, 24, 50);
    const availW = Math.max(1, w - padLeft - padRight);
    const availH = Math.max(1, h - padY * 2);
    const boundsFloor = NODE_RADIUS * 6;
    const bw = Math.max(boundsFloor, maxX - minX);
    const bh = Math.max(boundsFloor, maxY - minY);
    const zoom = clamp(Math.min(availW / bw, availH / bh), 0.15, 3);
    view.zoom = Number.isFinite(zoom) && zoom > 0 ? zoom : 1;
    view.x = padLeft + (availW - bw * view.zoom) / 2 - minX * view.zoom;
    view.y = h / 2 - ((minY + maxY) / 2) * view.zoom;
    render();
  }

  let alpha = 1;
  const alphaDecay = 0.025;
  const alphaMin = 0.01;
  let rafId = null;

  function stepOnce() {
    const n = sim.length;
    for (let i = 0; i < n; i++) {
      const a = sim[i];
      for (let j = i + 1; j < n; j++) {
        const b = sim[j];
        const dx = a.x - b.x;
        const dy = a.y - b.y;
        const distSq = Math.max(dx * dx + dy * dy, 4);
        const dist = Math.sqrt(distSq);
        const force = REPULSION / distSq;
        const fx = (dx / dist) * force;
        const fy = (dy / dist) * force;
        if (a.fx == null) { a.vx += fx; a.vy += fy; }
        if (b.fx == null) { b.vx -= fx; b.vy -= fy; }
      }
    }
    for (const e of simEdges) {
      const dx = e.b.x - e.a.x;
      const dy = e.b.y - e.a.y;
      const dist = Math.sqrt(dx * dx + dy * dy) || 0.01;
      const target = LINK_DISTANCE;
      const f = (dist - target) * 0.02;
      const fx = (dx / dist) * f;
      const fy = (dy / dist) * f;
      if (e.a.fx == null) { e.a.vx += fx; e.a.vy += fy; }
      if (e.b.fx == null) { e.b.vx -= fx; e.b.vy -= fy; }
    }
    let maxSpeed = 0;
    for (const s of sim) {
      if (s.fx != null) {
        s.x = s.fx;
        s.y = s.fy;
        s.vx = 0;
        s.vy = 0;
        continue;
      }
      s.vx += -s.x * 0.0025;
      s.vy += -s.y * 0.0025;
      s.vx *= 0.82;
      s.vy *= 0.82;
      s.x += s.vx * alpha;
      s.y += s.vy * alpha;
      maxSpeed = Math.max(maxSpeed, Math.abs(s.vx), Math.abs(s.vy));
    }
    alpha = Math.max(0, alpha - alphaDecay);
    return maxSpeed;
  }

  function tick() {
    const maxSpeed = stepOnce();
    render();
    if (alpha > alphaMin && maxSpeed > 0.03) {
      rafId = requestAnimationFrame(tick);
    } else {
      rafId = null;
      if (!userAdjustedView) fit();
    }
  }

  function runSync() {
    let maxSpeed = Infinity;
    let guard = 0;
    while (alpha > alphaMin && maxSpeed > 0.03 && guard < 5000) {
      maxSpeed = stepOnce();
      guard += 1;
    }
    render();
    if (!userAdjustedView) fit();
  }

  function restart(minAlpha) {
    alpha = Math.max(alpha, minAlpha);
    if (reducedMotion()) {
      if (rafId) {
        cancelAnimationFrame(rafId);
        rafId = null;
      }
      runSync();
    } else if (!rafId) {
      rafId = requestAnimationFrame(tick);
    }
  }

  function startNodeDrag(ev, s, g) {
    ev.stopPropagation();
    ev.preventDefault();
    g.setPointerCapture(ev.pointerId);
    const start = screenToGraph(ev.clientX, ev.clientY);
    let moved = false;
    s.fx = s.x;
    s.fy = s.y;

    function onMove(e2) {
      const p = screenToGraph(e2.clientX, e2.clientY);
      if (Math.hypot(p.x - start.x, p.y - start.y) > 2) moved = true;
      s.fx = p.x;
      s.fy = p.y;
      restart(0.3);
    }
    function onUp() {
      g.removeEventListener('pointermove', onMove);
      g.removeEventListener('pointerup', onUp);
      if (!moved) {
        selected = s.data.pubkey;
        updateSelection();
        if (onSelect) onSelect(s.data);
      }
    }
    g.addEventListener('pointermove', onMove);
    g.addEventListener('pointerup', onUp);
  }

  let panState = null;
  svg.addEventListener('pointerdown', (ev) => {
    if (ev.target !== svg) return;
    panState = { startX: ev.clientX, startY: ev.clientY, viewX: view.x, viewY: view.y };
    svg.setPointerCapture(ev.pointerId);
    userAdjustedView = true;
  });
  svg.addEventListener('pointermove', (ev) => {
    if (!panState) return;
    view.x = panState.viewX + (ev.clientX - panState.startX);
    view.y = panState.viewY + (ev.clientY - panState.startY);
    render();
  });
  svg.addEventListener('pointerup', () => {
    panState = null;
  });
  svg.addEventListener('wheel', (ev) => {
    ev.preventDefault();
    userAdjustedView = true;
    const rect = svg.getBoundingClientRect();
    const cx = ev.clientX - rect.left;
    const cy = ev.clientY - rect.top;
    const factor = ev.deltaY < 0 ? 1.12 : 0.89;
    const newZoom = clamp(view.zoom * factor, 0.15, 4);
    const gx = (cx - view.x) / view.zoom;
    const gy = (cy - view.y) / view.zoom;
    view.x = cx - gx * newZoom;
    view.y = cy - gy * newZoom;
    view.zoom = newZoom;
    render();
  }, { passive: false });

  fitBtn.addEventListener('click', () => {
    userAdjustedView = false;
    fit();
  });

  buildNodes();
  buildEdges();
  render();
  fit();
  if (!single) restart(1);

  let resizeObserver = null;
  if (typeof ResizeObserver === 'function') {
    resizeObserver = new ResizeObserver(() => {
      if (!userAdjustedView) fit();
    });
    resizeObserver.observe(wrap);
  }

  return {
    destroy() {
      if (rafId) cancelAnimationFrame(rafId);
      if (resizeObserver) resizeObserver.disconnect();
    },
    setSelected(pubkey) {
      selected = pubkey;
      updateSelection();
    },
    fit() {
      userAdjustedView = false;
      fit();
    },
  };
}
