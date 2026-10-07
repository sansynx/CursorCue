export function setupSmoothScroll(win, reducedMotion) {
  let frame = 0;
  let destination = 0;
  let previousTime = 0;
  let direction = 0;

  function stop() {
    win.cancelAnimationFrame(frame);
    frame = 0;
    direction = 0;
  }

  function advance(time) {
    const maximum = Math.max(
      0,
      win.document.documentElement.scrollHeight - win.innerHeight,
    );
    destination = Math.min(destination, maximum);
    const remaining = destination - win.scrollY;
    const elapsed = Math.min(time - previousTime, 64);
    previousTime = time;
    if (Math.abs(remaining) <= 2) {
      win.scrollTo({ top: destination, behavior: "instant" });
      stop();
      return;
    }
    const travel = Math.max(
      1,
      Math.abs(remaining) * (1 - Math.exp(-elapsed / 55)),
    );
    win.scrollTo({
      top: win.scrollY + Math.sign(remaining) * travel,
      behavior: "instant",
    });
    frame = win.requestAnimationFrame(advance);
  }

  win.addEventListener(
    "wheel",
    (event) => {
      // Preserve native precision scrolling, horizontal gestures, and browser zoom.
      if (
        reducedMotion.matches ||
        event.ctrlKey ||
        event.metaKey ||
        event.shiftKey ||
        !event.cancelable ||
        Math.abs(event.deltaX) > Math.abs(event.deltaY) ||
        (event.deltaMode === 0 && Math.abs(event.deltaY) < 40)
      ) {
        stop();
        return;
      }
      const unit =
        event.deltaMode === 1
          ? 20
          : event.deltaMode === 2
            ? win.innerHeight
            : 1;
      const delta = event.deltaY * unit;
      const nextDirection = Math.sign(delta);
      const maximum = Math.max(
        0,
        win.document.documentElement.scrollHeight - win.innerHeight,
      );
      const base =
        frame && direction === nextDirection ? destination : win.scrollY;
      const target = Math.max(0, Math.min(maximum, base + delta));
      if (target === win.scrollY) {
        stop();
        return;
      }
      destination = target;
      direction = nextDirection;
      event.preventDefault();
      if (!frame) {
        previousTime = win.performance.now();
        frame = win.requestAnimationFrame(advance);
      }
    },
    { passive: false },
  );

  for (const type of [
    "keydown",
    "touchstart",
    "pointerdown",
    "blur",
    "resize",
    "pagehide",
  ]) {
    win.addEventListener(type, stop, { passive: true });
  }
  reducedMotion.addEventListener("change", stop);
}
