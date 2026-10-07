import test from "node:test";
import assert from "node:assert/strict";

async function fixture(roundPixels = false) {
  const frames = new Map();
  let time = 0;
  let id = 0;
  const win = new EventTarget();
  const motion = new EventTarget();
  motion.matches = false;
  Object.assign(win, {
    scrollY: 0,
    innerHeight: 800,
    document: { documentElement: { scrollHeight: 2000 } },
    performance: { now: () => time },
    requestAnimationFrame: (callback) => {
      frames.set(++id, callback);
      return id;
    },
    cancelAnimationFrame: (frame) => frames.delete(frame),
    scrollTo: ({ top, behavior }) => {
      assert.equal(behavior, "instant");
      win.scrollY = roundPixels ? Math.round(top) : top;
    },
  });
  const { setupSmoothScroll } = await import("../dist/scroll.js");
  setupSmoothScroll(win, motion);
  function wheel(deltaY, extra = {}) {
    const event = new Event("wheel", { cancelable: true });
    Object.assign(event, { deltaY, deltaX: 0, deltaMode: 0, ...extra });
    win.dispatchEvent(event);
    return event;
  }
  function step(count = 1, interval = 16) {
    for (let i = 0; i < count; i++) {
      time += interval;
      const pending = [...frames.values()];
      frames.clear();
      pending.forEach((callback) => callback(time));
    }
  }
  return { win, motion, frames, wheel, step };
}

test("wheel movement eases toward its destination and stops scheduling frames", async () => {
  const { win, frames, wheel, step } = await fixture();
  assert.equal(wheel(120).defaultPrevented, true);
  assert.equal(win.scrollY, 0);
  step();
  assert.ok(win.scrollY > 0 && win.scrollY < 120);
  step(40);
  assert.equal(win.scrollY, 120);
  assert.equal(frames.size, 0);
});

test("rapid input keeps a single frame scheduled and clamps to page bounds", async () => {
  const { win, frames, wheel, step } = await fixture();
  for (let i = 0; i < 50; i++) wheel(120);
  assert.equal(frames.size, 1);
  step(50);
  assert.equal(win.scrollY, 1200);
  assert.equal(wheel(120).defaultPrevented, false);
});

test("reversing the wheel changes direction immediately", async () => {
  const { win, wheel, step } = await fixture();
  win.scrollY = 400;
  wheel(240);
  step(2);
  const before = win.scrollY;
  wheel(-120);
  step();
  assert.ok(win.scrollY < before);
});

test("trackpad, horizontal movement, zoom, and reduced motion use native input", async () => {
  const { motion, wheel, frames } = await fixture();
  assert.equal(wheel(8).defaultPrevented, false);
  assert.equal(wheel(120, { deltaX: 160 }).defaultPrevented, false);
  assert.equal(wheel(120, { ctrlKey: true }).defaultPrevented, false);
  motion.matches = true;
  assert.equal(wheel(120).defaultPrevented, false);
  assert.equal(frames.size, 0);
});

test("keyboard, touch, clicks, and preference changes cancel pending movement", async () => {
  for (const type of ["keydown", "touchstart", "pointerdown", "blur"]) {
    const { win, wheel, step, frames } = await fixture();
    wheel(120);
    win.dispatchEvent(new Event(type));
    step(30);
    assert.equal(win.scrollY, 0);
    assert.equal(frames.size, 0);
  }
  const { motion, wheel, frames } = await fixture();
  wheel(120);
  motion.matches = true;
  motion.dispatchEvent(new Event("change"));
  assert.equal(frames.size, 0);
});

test("line and page wheel units retain their expected travel distance", async () => {
  const { win, wheel, step } = await fixture();
  wheel(3, { deltaMode: 1 });
  step(40);
  assert.equal(win.scrollY, 60);
  wheel(1, { deltaMode: 2 });
  step(50);
  assert.equal(win.scrollY, 860);
});

test("pixel rounding cannot leave the scroll loop running at rest", async () => {
  const { win, frames, wheel, step } = await fixture(true);
  wheel(120);
  step(60);
  assert.equal(win.scrollY, 120);
  assert.equal(frames.size, 0);
});

test("high-refresh displays settle even when scrolling is rounded to pixels", async () => {
  const { win, frames, wheel, step } = await fixture(true);
  wheel(120);
  step(200, 4);
  assert.equal(win.scrollY, 120);
  assert.equal(frames.size, 0);
});
