import { el, isHttpUrl } from './util.js';

const TYPE_MS = 45;
const MARGIN = 4;
const TAIL_H = 6;
const TAIL_W = 11;
const SVG_NS = 'http://www.w3.org/2000/svg';

export const BALLOON_TEXT = {
  close: 'とじる',
  more: (n) => `ほか ${n} 件`,
};

function buildTail() {
  const svg = document.createElementNS(SVG_NS, 'svg');
  svg.setAttribute('class', 'desk-balloon-tail');
  svg.setAttribute('width', String(TAIL_W));
  svg.setAttribute('height', String(TAIL_H + 1));
  svg.setAttribute('viewBox', `0 0 ${TAIL_W} ${TAIL_H + 1}`);
  svg.setAttribute('shape-rendering', 'crispEdges');
  svg.setAttribute('aria-hidden', 'true');
  const rect = (x, y, w, fill) => {
    const r = document.createElementNS(SVG_NS, 'rect');
    r.setAttribute('x', String(x));
    r.setAttribute('y', String(y));
    r.setAttribute('width', String(w));
    r.setAttribute('height', '1');
    r.setAttribute('fill', fill);
    svg.append(r);
  };
  for (let r = 0; r <= TAIL_H; r += 1) {
    const x0 = r;
    const x1 = TAIL_W - 1 - r;
    if (x1 < x0) break;
    rect(x0, r, x1 - x0 + 1, '#000000');
    if (x1 - x0 >= 2) rect(x0 + 1, r, x1 - x0 - 1, '#ffffe1');
  }
  return svg;
}

export function createBalloon(container, { onClose } = {}) {
  const typed = el('span', { class: 'desk-balloon-typed', 'aria-hidden': 'true' });
  const rest = el('span', { class: 'desk-balloon-rest', 'aria-hidden': 'true' });
  const full = el('span', { class: 'desk-sr-only' });
  const text = el('p', { class: 'desk-balloon-text', dir: 'auto' }, [typed, rest, full]);
  const links = el('ul', { class: 'desk-balloon-links' });
  const more = el('p', { class: 'desk-balloon-more' });
  const closeBtn = el('button', { type: 'button', class: 'desk-dialog-btn desk-dialog-btn-sm desk-balloon-close' }, BALLOON_TEXT.close);
  const buttons = el('div', { class: 'desk-balloon-buttons' }, closeBtn);
  const tail = buildTail();
  const root = el('div', { class: 'desk-balloon', hidden: true }, [text, links, more, buttons, tail]);
  container.append(root);

  let current = null;
  let chars = [];
  let shownChars = -1;
  let size = null;
  let lastPos = '';

  function close(reason) {
    if (!current) return;
    const closing = current;
    current = null;
    root.hidden = true;
    const hadFocus = root.contains(document.activeElement);
    if (onClose) onClose(closing, reason, hadFocus);
  }

  closeBtn.addEventListener('click', (ev) => {
    ev.stopPropagation();
    close('button');
  });
  root.addEventListener('click', (ev) => {
    ev.stopPropagation();
    if (current && !current.closable && !(ev.target instanceof Element && ev.target.closest('a'))) close('click');
  });
  root.addEventListener('pointerdown', (ev) => ev.stopPropagation());

  function renderTyped(n) {
    if (n === shownChars) return;
    shownChars = n;
    typed.textContent = chars.slice(0, n).join('');
    rest.textContent = chars.slice(n).join('');
  }

  return {
    el: root,
    get open() {
      return current != null;
    },
    get content() {
      return current;
    },
    typing(now) {
      return current != null && !current.instant && now - current.start < chars.length * TYPE_MS;
    },
    show(content, now, { instant = false } = {}) {
      current = { ...content, start: now, instant };
      chars = Array.from(content.text);
      shownChars = -1;
      full.textContent = content.text;
      renderTyped(instant ? chars.length : 0);
      links.replaceChildren(
        ...(content.links || []).map(({ label, href, onOpen }) => {
          if (!isHttpUrl(href)) return el('li', {}, el('span', { dir: 'auto' }, label));
          const a = el('a', { class: 'desk-balloon-link', href, target: '_blank', rel: 'noopener noreferrer', dir: 'auto', title: label }, label);
          const opened = () => {
            if (onOpen) onOpen();
          };
          a.addEventListener('click', opened);
          a.addEventListener('auxclick', (ev) => {
            if (ev.button === 1) opened();
          });
          return el('li', {}, a);
        }),
      );
      links.hidden = links.childElementCount === 0;
      more.textContent = content.more ? BALLOON_TEXT.more(content.more) : '';
      more.hidden = !content.more;
      buttons.hidden = !content.closable;
      root.hidden = false;
      size = null;
      lastPos = '';
    },
    close,
    remeasure() {
      size = null;
    },
    tick(now, box, bounds) {
      if (!current) return;
      renderTyped(current.instant ? chars.length : Math.min(chars.length, Math.floor((now - current.start) / TYPE_MS)));
      if (current.autoCloseMs != null && now - current.start >= chars.length * (current.instant ? 0 : TYPE_MS) + current.autoCloseMs) {
        close('timeout');
        return;
      }
      if (!size) size = { w: root.offsetWidth, h: root.offsetHeight };
      const cx = box.pointX;
      const left = Math.max(MARGIN, Math.min(bounds.width - size.w - MARGIN, cx - Math.floor(size.w / 2)));
      let top = box.pointY - TAIL_H - size.h;
      let below = false;
      if (top < MARGIN) {
        below = true;
        top = Math.min(box.bottom + TAIL_H, bounds.ground - size.h - MARGIN);
      }
      top = Math.max(MARGIN, top);
      const tailLeft = Math.max(4, Math.min(size.w - TAIL_W - 4, cx - left - Math.floor(TAIL_W / 2)));
      const pos = `${left},${top},${below},${tailLeft}`;
      if (pos === lastPos) return;
      lastPos = pos;
      root.style.left = `${left}px`;
      root.style.top = `${top}px`;
      root.dataset.placement = below ? 'below' : 'above';
      tail.style.left = `${tailLeft}px`;
    },
  };
}
