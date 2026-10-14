process.env.TZ = 'UTC';

class MemoryStorage {
  #items = new Map();
  getItem(key) {
    return this.#items.has(key) ? this.#items.get(key) : null;
  }
  setItem(key, value) {
    this.#items.set(key, String(value));
  }
  removeItem(key) {
    this.#items.delete(key);
  }
  clear() {
    this.#items.clear();
  }
}

class FakeDocument extends EventTarget {
  documentElement = {};
  hidden = false;
  getElementById() {
    return null;
  }
  querySelector() {
    return null;
  }
  querySelectorAll() {
    return [];
  }
}

const define = (name, value) => Object.defineProperty(globalThis, name, { value, configurable: true, writable: true });

define('localStorage', new MemoryStorage());
define('document', new FakeDocument());
define('navigator', { language: 'en-US' });

export function resetStorage(lang = 'en') {
  localStorage.clear();
  localStorage.setItem('swing:lang', lang);
}

resetStorage();
