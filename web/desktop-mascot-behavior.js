const IDLE_WAIT = [1500, 6000];
const WALK_CHANCE = 0.55;
const MIN_WALK = 24;
const SLEEP_AFTER = [60000, 120000];
const SLEEP_LENGTH = [30000, 90000];
const SURPRISE_MS = 900;
const GRAVITY = 2400;
const MAX_STEP_MS = 100;

export function between([lo, hi], random = Math.random) {
  return lo + random() * (hi - lo);
}

export function createBehavior(pack, { random = Math.random, now = 0 } = {}) {
  const s = pack.scale;
  const extent = Math.max(pack.anchor.x, pack.frame.width - pack.anchor.x) * s;
  const top = pack.anchor.y * s;
  const bounds = { width: 0, ground: 0 };
  const st = {
    mode: 'idle',
    x: 0,
    y: 0,
    vy: 0,
    facing: pack.facing,
    target: 0,
    nextDecision: now + between(IDLE_WAIT, random),
    sleepAt: now + between(SLEEP_AFTER, random),
    wakeAt: 0,
    surpriseUntil: 0,
    talking: false,
    reduced: false,
    walk: true,
    placed: false,
  };

  function minX() {
    return Math.min(extent, bounds.width / 2);
  }

  function maxX() {
    return Math.max(bounds.width - extent, bounds.width / 2);
  }

  function clampX(x) {
    return Math.min(maxX(), Math.max(minX(), x));
  }

  function toIdle(t) {
    st.mode = 'idle';
    st.nextDecision = t + between(IDLE_WAIT, random);
  }

  function interact(t) {
    st.sleepAt = t + between(SLEEP_AFTER, random);
  }

  function decide(t) {
    if (t >= st.sleepAt) {
      st.mode = 'sleep';
      st.wakeAt = t + between(SLEEP_LENGTH, random);
      return;
    }
    if (st.reduced || !st.walk || random() >= WALK_CHANCE) {
      st.nextDecision = t + between(IDLE_WAIT, random);
      return;
    }
    const target = minX() + random() * (maxX() - minX());
    if (Math.abs(target - st.x) < MIN_WALK) {
      st.nextDecision = t + between(IDLE_WAIT, random);
      return;
    }
    st.target = target;
    st.facing = target > st.x ? 'right' : 'left';
    st.mode = 'walk';
  }

  function animation() {
    if (st.mode === 'drag' || st.mode === 'fall' || st.mode === 'surprise') return st.mode;
    if (st.talking) return 'talk';
    return st.mode;
  }

  function walk(dt, step) {
    const dist = pack.walkMoves ? Math.abs(step.dx) * s : (pack.speed * s * dt) / 1000;
    const dir = st.target > st.x ? 1 : -1;
    const remaining = Math.abs(st.target - st.x);
    if (dist >= remaining) {
      st.x = st.target;
      return true;
    }
    st.x += dir * dist;
    return false;
  }

  return {
    get x() {
      return st.x;
    },
    get y() {
      return st.y;
    },
    get facing() {
      return st.facing;
    },
    get mode() {
      return st.mode;
    },
    get animation() {
      return animation();
    },
    get held() {
      return st.mode === 'drag';
    },
    setReduced(value) {
      st.reduced = value;
      if (value && (st.mode === 'walk' || st.mode === 'fall')) {
        st.y = bounds.ground;
        st.mode = 'idle';
      }
    },
    setWalk(value, t) {
      st.walk = value;
      if (!value && st.mode === 'walk') toIdle(t);
    },
    setBounds(width, ground, t) {
      bounds.width = width;
      bounds.ground = ground;
      if (!st.placed) {
        st.placed = true;
        st.x = minX() + random() * (maxX() - minX());
        st.y = ground;
        st.facing = random() < 0.5 ? 'left' : 'right';
        return;
      }
      st.x = clampX(st.x);
      if (st.mode === 'walk') st.target = clampX(st.target);
      if (st.mode !== 'drag' && st.mode !== 'fall') st.y = ground;
      if (st.y > ground) st.y = ground;
      if (st.mode === 'fall' && st.reduced) toIdle(t);
    },
    update(t, dtMs, step) {
      const dt = Math.min(dtMs, MAX_STEP_MS);
      switch (st.mode) {
        case 'idle':
          if (!st.talking && t >= st.nextDecision) decide(t);
          break;
        case 'walk':
          if (st.talking) toIdle(t);
          else if (walk(dt, step)) toIdle(t);
          break;
        case 'sleep':
          if (t >= st.wakeAt) {
            interact(t);
            toIdle(t);
          }
          break;
        case 'surprise':
          if (t >= st.surpriseUntil) toIdle(t);
          break;
        case 'fall':
          st.vy += (GRAVITY * dt) / 1000;
          st.y += (st.vy * dt) / 1000;
          if (st.y >= bounds.ground) {
            st.y = bounds.ground;
            st.vy = 0;
            toIdle(t);
          }
          break;
        default:
          break;
      }
      if (!pack.animations[animation()].flip) st.facing = pack.facing;
    },
    surprise(t) {
      if (st.mode === 'drag' || st.mode === 'fall') return;
      interact(t);
      st.mode = 'surprise';
      st.surpriseUntil = t + SURPRISE_MS;
    },
    wake(t) {
      interact(t);
      if (st.mode === 'sleep') toIdle(t);
    },
    setTalking(value, t) {
      st.talking = value;
      interact(t);
      if (value && st.mode === 'sleep') toIdle(t);
      if (value && st.mode === 'walk') toIdle(t);
    },
    grab(t) {
      interact(t);
      st.mode = 'drag';
      st.vy = 0;
    },
    dragTo(x, y) {
      st.x = clampX(x);
      st.y = Math.min(bounds.ground, Math.max(top, y));
    },
    release(t) {
      if (st.mode !== 'drag') return;
      if (st.reduced || st.y >= bounds.ground) {
        st.y = bounds.ground;
        toIdle(t);
        return;
      }
      st.mode = 'fall';
      st.vy = 0;
    },
  };
}
