function clamp(v, lo, hi) {
  return Math.min(hi, Math.max(lo, v));
}

/* Win95-style listbox popup: a plain `<select>` can't be restyled to match, so this is a hand-rolled combobox. */
export function createCombobox({ field, list, onChange }) {
  const options = Array.from(list.querySelectorAll('[role="option"]'));
  const valueEl = field.querySelector('.desk-wp-combobox-value');
  let open = false;
  let activeIndex = 0;

  function indexOf(value) {
    const i = options.findIndex((o) => o.dataset.value === value);
    return i === -1 ? 0 : i;
  }

  function render(value) {
    field.dataset.value = value;
    const opt = options.find((o) => o.dataset.value === value);
    if (valueEl) valueEl.textContent = opt ? opt.textContent : '';
    for (const o of options) o.setAttribute('aria-selected', String(o.dataset.value === value));
  }

  function position() {
    const rect = field.getBoundingClientRect();
    list.style.left = `${Math.round(rect.left)}px`;
    list.style.top = `${Math.round(rect.bottom + 2)}px`;
    list.style.width = `${Math.round(rect.width)}px`;
  }

  function highlight(index) {
    activeIndex = clamp(index, 0, options.length - 1);
    for (let i = 0; i < options.length; i += 1) options[i].classList.toggle('is-active', i === activeIndex);
    const active = options[activeIndex];
    field.setAttribute('aria-activedescendant', active.id);
    active.scrollIntoView({ block: 'nearest' });
  }

  function onOutsideClick(ev) {
    if (field.contains(ev.target) || list.contains(ev.target)) return;
    close();
  }

  function openList() {
    if (field.disabled || open) return;
    open = true;
    position();
    list.hidden = false;
    field.setAttribute('aria-expanded', 'true');
    highlight(indexOf(field.dataset.value));
    document.addEventListener('click', onOutsideClick, true);
  }

  function close() {
    if (!open) return;
    open = false;
    list.hidden = true;
    field.setAttribute('aria-expanded', 'false');
    field.removeAttribute('aria-activedescendant');
    document.removeEventListener('click', onOutsideClick, true);
  }

  function commit(value) {
    render(value);
    close();
    onChange(value);
  }

  function handleKey(ev) {
    switch (ev.key) {
      case 'Escape':
        ev.preventDefault();
        ev.stopPropagation();
        close();
        return;
      case 'ArrowDown':
        ev.preventDefault();
        highlight(activeIndex + 1);
        return;
      case 'ArrowUp':
        ev.preventDefault();
        highlight(activeIndex - 1);
        return;
      case 'Home':
        ev.preventDefault();
        highlight(0);
        return;
      case 'End':
        ev.preventDefault();
        highlight(options.length - 1);
        return;
      case 'Enter':
      case ' ':
        ev.preventDefault();
        commit(options[activeIndex].dataset.value);
        return;
      default:
    }
  }

  field.addEventListener('click', () => {
    if (open) close();
    else openList();
  });
  field.addEventListener('keydown', (ev) => {
    if (open) return;
    if (ev.key === 'ArrowDown' || ev.key === 'ArrowUp') {
      ev.preventDefault();
      openList();
    }
  });
  for (const opt of options) {
    opt.addEventListener('click', () => commit(opt.dataset.value));
    opt.addEventListener('mouseenter', () => highlight(options.indexOf(opt)));
  }

  return {
    setValue(value) {
      render(value);
    },
    setDisabled(disabled) {
      field.disabled = disabled;
      if (disabled) close();
    },
    isOpen() {
      return open;
    },
    close,
    handleKey,
  };
}
