import { clamp, sanitizeDisplayText } from './util.js';
import { EMOJI_SPRITE, fingerprintIds } from './ui.js';

const NODE_RADIUS = 10;
const NODE_EMOJI_SIZE = 14;
const ARROW_LEN = 8;
const LABEL_MAX = 16;
const LINK_DISTANCE = 90;
const REPULSION = 1300;
const RING_SPACING = 60;
const LABEL_GAP = 4;
const NODE_STROKE_ALLOWANCE = 2;
const LABEL_HEIGHT = 14;
const LABEL_CHAR_PX = 6.6;
const COLLIDE_PAD = 6;
const EDGE_CLEARANCE = 22;
const EDGE_AVOID_MAX_NODES = 150;
const WARMUP_MS = 150;
const LABEL_SCALE_PASSES = 3;
const LABEL_SCALE_ALPHA = 0.3;
const SVG_NS = 'http://www.w3.org/2000/svg';

function truncateLabel(label) {
  if (label.length <= LABEL_MAX) return label;
  return label.slice(0, LABEL_MAX - 1) + '…';
}

function moreSuffix(node) {
  return node.more_sites > 0 ? ` +${node.more_sites}` : '';
}

function buildNodeEmoji(node) {
  const [id] = fingerprintIds(node.fingerprint, 1);
  if (!id) return null;
  const use = document.createElementNS(SVG_NS, 'use');
  use.setAttribute('class', 'swing-node-emoji');
  use.setAttribute('href', `${EMOJI_SPRITE}#e-${id}`);
  use.setAttribute('x', String(-NODE_EMOJI_SIZE / 2));
  use.setAttribute('y', String(-NODE_EMOJI_SIZE / 2));
  use.setAttribute('width', String(NODE_EMOJI_SIZE));
  use.setAttribute('height', String(NODE_EMOJI_SIZE));
  use.setAttribute('aria-hidden', 'true');
  return use;
}

function circularMean(angles) {
  let sin = 0;
  let cos = 0;
  for (const a of angles) {
    sin += Math.sin(a);
    cos += Math.cos(a);
  }
  return Math.atan2(sin, cos);
}

function seedPositions(sim) {
  const rings = new Map();
  for (const s of sim) {
    const depth = Number.isFinite(s.data.depth) ? s.data.depth : 0;
    if (!rings.has(depth)) rings.set(depth, []);
    rings.get(depth).push(s);
  }
  const angles = new Map();
  let radius = 0;
  for (const depth of [...rings.keys()].sort((a, b) => a - b)) {
    const ring = rings.get(depth);
    const minRadius = ring.length > 1 ? (ring.length * RING_SPACING) / (Math.PI * 2) : 0;
    radius = depth === 0 && ring.length === 1 ? 0 : Math.max(radius + LINK_DISTANCE, minRadius);
    const step = (Math.PI * 2) / ring.length;
    const wanted = ring.map((s, i) => {
      const placed = s.neighbors.filter((n) => angles.has(n)).map((n) => angles.get(n));
      return { s, angle: placed.length > 0 ? circularMean(placed) : i * step };
    });
    wanted.sort((a, b) => a.angle - b.angle);
    const offset = circularMean(wanted.map((w, i) => w.angle - i * step));
    wanted.forEach((w, i) => {
      const angle = offset + i * step;
      angles.set(w.s, angle);
      w.s.x = Math.cos(angle) * radius;
      w.s.y = Math.sin(angle) * radius;
    });
  }
}

function labelSide(s, scale) {
  let right = 0;
  let left = 0;
  for (const n of s.neighbors) {
    const dx = n.x - s.x;
    if (dx === 0) continue;
    const slope = Math.abs(n.y - s.y) / Math.abs(dx);
    const overlap = Math.min(s.labelWidth * scale, (LABEL_HEIGHT * scale) / 2 / Math.max(slope, 0.001));
    if (dx > 0) right += overlap;
    else left += overlap;
  }
  return left < right ? 'left' : 'right';
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
  const sim = nodes.map((n) => ({
    data: n,
    x: 0,
    y: 0,
    vx: 0,
    vy: 0,
    fx: null,
    fy: null,
    labelWidth: 0,
    side: 'right',
    box: null,
    neighbors: [],
  }));
  const byId = new Map(sim.map((s) => [s.data.pubkey, s]));
  const simEdges = edges
    .map((e) => ({ edge: e, a: byId.get(e.from), b: byId.get(e.to) }))
    .filter((e) => e.a && e.b && e.a !== e.b);
  for (const e of simEdges) {
    e.a.neighbors.push(e.b);
    e.b.neighbors.push(e.a);
  }
  seedPositions(sim);

  const view = { x: 0, y: 0, zoom: 1 };
  let selected = selectedPubkey || null;
  let userAdjustedView = false;
  const nodeEls = new Map();
  const labelEls = [];
  let baseLabelFontPx = null;
  let labelScale = 1;

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
      const label = sanitizeDisplayText(s.data.label);
      const more = moreSuffix(s.data);
      g.setAttribute('tabindex', '0');
      g.setAttribute('role', 'button');
      g.setAttribute('aria-label', label + more);

      const circle = document.createElementNS(SVG_NS, 'circle');
      circle.setAttribute('r', String(NODE_RADIUS));

      const title = document.createElementNS(SVG_NS, 'title');
      title.textContent = label + more;

      const text = document.createElementNS(SVG_NS, 'text');
      text.setAttribute('x', String(NODE_RADIUS + LABEL_GAP));
      text.setAttribute('y', '4');
      text.textContent = truncateLabel(label) + more;

      g.append(circle);
      const emoji = buildNodeEmoji(s.data);
      if (emoji) g.append(emoji);
      g.append(title, text);
      labelEls.push(text);
      s.label = text;
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

  function measureLabels() {
    for (const s of sim) {
      let width = 0;
      try {
        width = s.label.getComputedTextLength();
      } catch {
        width = 0;
      }
      s.labelWidth = width > 0 ? width : s.label.textContent.length * LABEL_CHAR_PX;
    }
  }

  function applyLabelScale() {
    if (labelEls.length === 0) return;
    if (baseLabelFontPx == null) {
      const parsed = parseFloat(getComputedStyle(labelEls[0]).fontSize);
      baseLabelFontPx = Number.isFinite(parsed) && parsed > 0 ? parsed : 11;
    }
    const scale = 1 / clamp(view.zoom, 0.15, 4);
    const fontSize = `${(baseLabelFontPx * scale).toFixed(2)}px`;
    const halo = `${(3 * scale).toFixed(2)}px`;
    for (const text of labelEls) {
      text.style.fontSize = fontSize;
      text.style.strokeWidth = halo;
    }
  }

  function placeLabel(s) {
    const offset = NODE_RADIUS + NODE_STROKE_ALLOWANCE + LABEL_GAP / clamp(view.zoom, 0.15, 4);
    const left = s.side === 'left';
    s.label.setAttribute('x', String(left ? -offset : offset));
    s.label.setAttribute('text-anchor', left ? 'end' : 'start');
  }

  function render() {
    applyLabelScale();
    for (const s of sim) {
      const g = nodeEls.get(s.data.pubkey);
      if (g) g.setAttribute('transform', `translate(${s.x.toFixed(1)},${s.y.toFixed(1)})`);
      if (s.label) placeLabel(s);
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
    let leftLabel = 0;
    let rightLabel = 0;
    for (const s of sim) {
      if (s.side === 'left') leftLabel = Math.max(leftLabel, s.labelWidth);
      else rightLabel = Math.max(rightLabel, s.labelWidth);
    }
    const edgePad = clamp(w * 0.04, 16, 40);
    const labelPad = (width) => (width > 0 ? NODE_RADIUS + LABEL_GAP + width : NODE_RADIUS);
    const padLeft = Math.min(w * 0.3, edgePad + labelPad(leftLabel));
    const padRight = Math.min(w * 0.3, edgePad + labelPad(rightLabel));
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
  const alphaDecay = 0.004;
  const alphaMin = 0.01;
  let rafId = null;

  function stepOnce() {
    const n = sim.length;
    for (const s of sim) s.box = nodeBox(s);
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
        collide(a, b);
      }
    }
    if (n <= EDGE_AVOID_MAX_NODES) avoidEdges();
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
    for (const s of sim) s.side = labelSide(s, labelScale);
    alpha = Math.max(0, alpha - alphaDecay);
    return maxSpeed;
  }

  function nodeBox(s) {
    const label = NODE_RADIUS + NODE_STROKE_ALLOWANCE + (LABEL_GAP + s.labelWidth) * labelScale;
    const half = Math.max(NODE_RADIUS, (LABEL_HEIGHT * labelScale) / 2);
    return s.side === 'left'
      ? { x0: s.x - label, x1: s.x + NODE_RADIUS, y0: s.y - half, y1: s.y + half }
      : { x0: s.x - NODE_RADIUS, x1: s.x + label, y0: s.y - half, y1: s.y + half };
  }

  function collide(a, b) {
    const ba = a.box;
    const bb = b.box;
    const ox = Math.min(ba.x1, bb.x1) - Math.max(ba.x0, bb.x0) + COLLIDE_PAD;
    const oy = Math.min(ba.y1, bb.y1) - Math.max(ba.y0, bb.y0) + COLLIDE_PAD;
    if (ox <= 0 || oy <= 0) return;
    let px = 0;
    let py = 0;
    if (oy <= ox) py = (a.y < b.y ? -1 : 1) * oy * 0.25;
    else px = ((ba.x0 + ba.x1) < (bb.x0 + bb.x1) ? -1 : 1) * ox * 0.25;
    if (a.fx == null) { a.vx += px; a.vy += py; }
    if (b.fx == null) { b.vx -= px; b.vy -= py; }
  }

  function avoidEdges() {
    for (const e of simEdges) {
      const ex = e.b.x - e.a.x;
      const ey = e.b.y - e.a.y;
      const lenSq = ex * ex + ey * ey;
      if (lenSq < 1) continue;
      for (const c of sim) {
        if (c === e.a || c === e.b) continue;
        const t = ((c.x - e.a.x) * ex + (c.y - e.a.y) * ey) / lenSq;
        if (t <= 0 || t >= 1) continue;
        let dx = c.x - (e.a.x + ex * t);
        let dy = c.y - (e.a.y + ey * t);
        let dist = Math.sqrt(dx * dx + dy * dy);
        if (dist >= EDGE_CLEARANCE) continue;
        if (dist < 0.01) {
          const len = Math.sqrt(lenSq);
          dx = -ey / len;
          dy = ex / len;
          dist = 1;
        } else {
          dx /= dist;
          dy /= dist;
        }
        const push = (EDGE_CLEARANCE - dist) * 0.1;
        if (c.fx == null) { c.vx += dx * push; c.vy += dy * push; }
        if (e.a.fx == null) { e.a.vx -= dx * push * 0.5; e.a.vy -= dy * push * 0.5; }
        if (e.b.fx == null) { e.b.vx -= dx * push * 0.5; e.b.vy -= dy * push * 0.5; }
      }
    }
  }

  function warmUp(deadline) {
    let maxSpeed = Infinity;
    while (alpha > alphaMin && maxSpeed > 0.03 && performance.now() < deadline) {
      maxSpeed = stepOnce();
    }
    return alpha <= alphaMin || maxSpeed <= 0.03;
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
    labelScale = 1 / clamp(view.zoom, 0.15, 4);
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
  measureLabels();
  const deadline = performance.now() + WARMUP_MS;
  let settled = single || warmUp(deadline);
  render();
  fit();
  for (let pass = 0; pass < LABEL_SCALE_PASSES && settled && !single; pass += 1) {
    const next = 1 / clamp(view.zoom, 0.15, 4);
    if (Math.abs(next - labelScale) <= labelScale * 0.05) break;
    labelScale = next;
    alpha = Math.max(alpha, LABEL_SCALE_ALPHA);
    settled = warmUp(deadline);
    render();
    fit();
  }
  if (!settled) restart(0);

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
