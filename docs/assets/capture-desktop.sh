#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
out=${1:-$root/docs/assets/dashboard-desktop.webp}
still=${out%.webp}.png
base=http://127.0.0.1:18082
session=swing-capture
work=$(mktemp -d)

ab() { agent-browser --session "$session" "$@" >/dev/null; }
cleanup() {
  agent-browser --session "$session" close >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT

cat >"$work/init.js" <<'EOF'
// The recorder's own cursor is not Windows-like, so the page draws a pixel-art one; frames forward their pointer to the top window.
if (window.top !== window) {
  const forward = (ev) => {
    const r = window.frameElement.getBoundingClientRect();
    window.top.__swingCursor?.(ev.type, ev.clientX + r.left, ev.clientY + r.top);
  };
  for (const type of ['pointermove', 'pointerdown', 'pointerup']) window.addEventListener(type, forward, true);
} else {
  const shapes = {
    default: [0, 0, `
K...........
KK..........
KWK.........
KWWK........
KWWWK.......
KWWWWK......
KWWWWWK.....
KWWWWWWK....
KWWWWWWWK...
KWWWWWWWWK..
KWWWWWWWWWK.
KWWWWWWKKKKK
KWWWKWWK....
KWWK.KWWK...
KWK..KWWK...
KK....KWWK..
K.....KWWK..
.......KWWK.
.......KWWK.
........KK..`],
    pointer: [5, 0, `
.....KK.........
....KWWK........
....KWWK........
....KWWK........
....KWWK........
....KWWKKK......
....KWWKWWKKK...
....KWWKWWKWWKK.
.KK.KWWKWWKWWKWK
KWWKKWWWWWWWWKWK
KWWWKWWWWWWWWWWK
.KWWKWWWWWWWWWWK
..KWWWWWWWWWWWWK
..KWWWWWWWWWWWK.
...KWWWWWWWWWWK.
...KWWWWWWWWWK..
....KWWWWWWWWK..
....KWWWWWWWWK..
....KKKKKKKKKK..`],
    grab: [8, 8, `
.......KK.......
...KK.KWWKKK....
..KWWKKWWKWWK...
..KWWKKWWKWWK.K.
...KWWKWWKWWKKWK
...KWWKWWKWWKWWK
.KK.KWWWWWWWKWWK
KWWKKWWWWWWWWWWK
KWWWKWWWWWWWWWK.
.KWWWWWWWWWWWWK.
..KWWWWWWWWWWWK.
..KWWWWWWWWWWK..
...KWWWWWWWWWK..
....KWWWWWWWK...
.....KWWWWWWK...
.....KWWWWWWK...`],
    grabbing: [8, 8, `
....KK.KK.KK....
...KWWKWWKWWKK..
...KWWWWWWWWKWK.
....KWWWWWWWWWK.
...KKWWWWWWWWWK.
..KWWWWWWWWWWWK.
..KWWWWWWWWWWWK.
...KWWWWWWWWWK..
....KWWWWWWWWK..
.....KWWWWWWK...
.....KWWWWWWK...`, 4],
    'ns-resize': [4, 7, `
....K....
...KWK...
..KWWWK..
.KWWWWWK.
KKKKWKKKK
...KWK...
...KWK...
...KWK...
...KWK...
...KWK...
KKKKWKKKK
.KWWWWWK.
..KWWWK..
...KWK...
....K....`],
    'nwse-resize': [6, 6, `
KKKKKKK......
KWWWWK.......
KWWWK........
KWWWWK.......
KWKWWWK......
KK.KWWWK.....
K...KWWWK...K
.....KWWWK.KK
......KWWWKWK
.......KWWWWK
........KWWWK
.......KWWWWK
......KKKKKKK`],
  };
  const rows = (s) => s.trim().split('\n');
  const transpose = (s) => rows(s)[0].split('').map((_, x) => rows(s).map((r) => r[x]).join('')).join('\n');
  const mirror = (s) => rows(s).map((r) => [...r].reverse().join('')).join('\n');
  shapes['ew-resize'] = [7, 4, transpose(shapes['ns-resize'][2])];
  shapes['nesw-resize'] = [6, 6, mirror(shapes['nwse-resize'][2])];
  const images = {};
  for (const [name, [hx, hy, art, top = 0]] of Object.entries(shapes)) {
    const lines = rows(art);
    const c = document.createElement('canvas');
    c.width = lines[0].length;
    c.height = lines.length + top;
    const g = c.getContext('2d');
    lines.forEach((line, y) => [...line].forEach((p, x) => {
      if (p === '.') return;
      g.fillStyle = p === 'K' ? '#000' : '#fff';
      g.fillRect(x, y + top, 1, 1);
    }));
    images[name] = { hx, hy, w: c.width, h: c.height, url: `url(${c.toDataURL()})` };
  }
  const cursor = document.createElement('div');
  cursor.style.cssText = 'position:fixed;left:0;top:0;z-index:2147483647;pointer-events:none;display:none;background-repeat:no-repeat;image-rendering:pixelated';
  let x = 0, y = 0, held = null, shown = '';
  const hovered = () => {
    let doc = document, px = x, py = y, el;
    for (;;) {
      el = doc.elementFromPoint(px, py);
      if (!(el instanceof HTMLIFrameElement) || !el.contentDocument) break;
      const r = el.getBoundingClientRect();
      doc = el.contentDocument;
      px -= r.left;
      py -= r.top;
    }
    const kw = el ? getComputedStyle(el).cursor : 'default';
    return images[kw] ? kw : 'default';
  };
  window.__swingCursor = (type, cx, cy) => {
    x = cx;
    y = cy;
    if (type === 'pointerdown') held = hovered() === 'grab' ? 'grabbing' : hovered();
    if (type === 'pointerup') held = null;
    if (!cursor.isConnected) document.documentElement.append(cursor);
    cursor.style.display = '';
  };
  for (const type of ['pointermove', 'pointerdown', 'pointerup']) {
    window.addEventListener(type, (ev) => window.__swingCursor(ev.type, ev.clientX, ev.clientY), true);
  }
  const draw = () => {
    const name = held ?? hovered();
    const img = images[name];
    if (name !== shown) {
      Object.assign(cursor.style, { width: `${img.w}px`, height: `${img.h}px`, backgroundImage: img.url });
      shown = name;
    }
    cursor.style.transform = `translate(${x - img.hx}px, ${y - img.hy}px)`;
    requestAnimationFrame(draw);
  };
  requestAnimationFrame(draw);

  localStorage.setItem('swing:lang', 'ja');
  localStorage.setItem('swing:nav:collapsed', '1');
  // Keeps both the mascots' spawn points and walk targets in the left 45% so they never pass under the dialog.
  let a = 7;
  Math.random = () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return (((t ^ (t >>> 14)) >>> 0) / 4294967296) * 0.45;
  };
}
EOF

login=$("$root/docker/demo/demo.sh" exec -T mirror swing dashboard open --no-browser | grep -oE 'http://[^ ]+' | tail -1)

ab open --init-script "$work/init.js"
ab set viewport 1280 800
ab open "$login"
ab open "$base/#/desktop"
ab mouse move 700 450
sleep 4

ab record start "$work/take.webm"
sleep 2
mascot_at='(() => { const r = document.querySelector(".desk-mascot").getBoundingClientRect(); return `${Math.round(r.x + r.width / 2)} ${Math.round(r.y + r.height * 0.6)}`; })()'
read -r mx my < <(agent-browser --session "$session" eval "$mascot_at" | tr -d '"')
ab mouse move "$mx" "$my" --duration 600 --steps 25
read -r mx my < <(agent-browser --session "$session" eval "$mascot_at" | tr -d '"')
ab mouse move "$mx" "$my" --duration 150 --steps 6
sleep 0.2
ab mouse down left
ab mouse move "$((mx + 40))" "$((my - 160))" --duration 700 --steps 30
sleep 0.8
ab mouse up left
ab mouse move 700 450 --duration 600 --steps 25
sleep 2
read -r ix iy < <(agent-browser --session "$session" eval \
  '(() => { const r = document.getElementById("desk-icon-control-panel").getBoundingClientRect(); return `${Math.round(r.x + r.width / 2)} ${Math.round(r.y + r.height / 3)}`; })()' | tr -d '"')
ab mouse move "$ix" "$iy" --duration 800 --steps 30
sleep 0.3
ab dblclick '#desk-icon-control-panel'
sleep 1.2
read -r x y tx ty < <(agent-browser --session "$session" eval \
  '(() => { const r = document.querySelector(".desk-dialog").getBoundingClientRect(); const x = Math.round(r.x + 140), y = Math.round(r.y + 12); return `${x} ${y} ${x + 804 - Math.round(r.x)} ${y + 196 - Math.round(r.y)}`; })()' | tr -d '"')
ab mouse move "$x" "$y" --duration 500 --steps 20
sleep 0.2
ab mouse down left
ab mouse move "$tx" "$ty" --duration 1200 --steps 50
ab mouse up left
ab mouse move 1000 450 --duration 600 --steps 25
sleep 5
ab record stop

marquee_shown='(() => { const f = document.getElementById("desk-page-frame"); const d = f.contentDocument; const c = d.querySelector(".desk-marquee").getBoundingClientRect(); const t = d.getElementById("desk-marquee-text").getBoundingClientRect(); const edge = document.querySelector(".desk-dialog").getBoundingClientRect().left - f.getBoundingClientRect().left; return t.left >= c.left && t.right <= Math.min(c.right, edge); })()'
for _ in $(seq 100); do
  if [ "$(agent-browser --session "$session" eval "$marquee_shown")" != true ]; then
    sleep 0.2
    continue
  fi
  ab screenshot "$still"
  green=$(ffmpeg -v error -i "$still" -vf crop=1:1:350:414 -f rawvideo -pix_fmt rgb24 - | od -An -tu1 | awk '{print $2}')
  [ "$green" -lt 100 ] && break
  sleep 0.3
done

ffmpeg -v error -y -ss 1 -i "$work/take.webm" -vf fps=10 \
  -c:v libwebp_anim -quality 80 -compression_level 6 -loop 0 "$out"
echo "$out" "$still"
