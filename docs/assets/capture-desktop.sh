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
if (window.top === window) {
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
sleep 3
ab dblclick '#desk-icon-control-panel'
sleep 1.2
read -r x y tx ty < <(agent-browser --session "$session" eval \
  '(() => { const r = document.querySelector(".desk-dialog").getBoundingClientRect(); const x = Math.round(r.x + 140), y = Math.round(r.y + 12); return `${x} ${y} ${x + 804 - Math.round(r.x)} ${y + 196 - Math.round(r.y)}`; })()' | tr -d '"')
ab mouse move "$x" "$y"
ab mouse down left
ab mouse move "$tx" "$ty" --duration 1200 --steps 50
ab mouse up left
ab mouse move 1000 450
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
