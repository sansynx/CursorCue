import test from "node:test";
import assert from "node:assert/strict";
import { CueDemo } from "../dist/demo-core.js";

test("freezing keeps the audience still while the physical cursor moves", () => {
  const cue = new CueDemo();
  cue.move(20, 30);
  cue.act("freeze");
  cue.move(80, 70);
  assert.deepEqual(cue.audience, { x: 20, y: 30 });
  assert.deepEqual(cue.real, { x: 80, y: 70 });
});
test("hide preserves the frozen mode and position when revealed", () => {
  const cue = new CueDemo();
  cue.move(25, 40);
  cue.act("freeze");
  cue.act("hide");
  cue.move(70, 80);
  assert.equal(cue.visible, false);
  cue.act("hide");
  assert.equal(cue.mode, "frozen");
  assert.deepEqual(cue.audience, { x: 25, y: 40 });
});
test("resume reveals and reconnects the audience cursor", () => {
  const cue = new CueDemo();
  cue.act("hide");
  cue.move(80, 60);
  cue.act("resume");
  assert.equal(cue.visible, true);
  assert.equal(cue.mode, "following");
  assert.deepEqual(cue.audience, { x: 80, y: 60 });
});

test("revealing a following cursor synchronizes to the current mouse position", () => {
  const cue = new CueDemo();
  cue.act("hide");
  cue.move(80, 70);
  cue.act("hide");
  assert.equal(cue.mode, "following");
  assert.deepEqual(cue.audience, { x: 80, y: 70 });
});
test("drop reveals at the real position and holds there", () => {
  const cue = new CueDemo();
  cue.act("hide");
  cue.move(70, 60);
  cue.act("drop");
  cue.move(5, 10);
  assert.equal(cue.visible, true);
  assert.deepEqual(cue.audience, { x: 70, y: 60 });
});
test("coordinates remain inside the preview", () => {
  const cue = new CueDemo();
  cue.move(-30, 150);
  assert.deepEqual(cue.real, { x: 0, y: 100 });
});
