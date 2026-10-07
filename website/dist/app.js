import { CueDemo } from "./demo-core.js";
import { setupSmoothScroll } from "./scroll.js";

const cue = new CueDemo();
const workspace = document.getElementById("demo-workspace");
const audience = document.getElementById("audience-pointer");
const privatePointer = document.getElementById("private-pointer");
const modeLabel = document.getElementById("mode-label");
const caption = document.getElementById("demo-caption");
const ripple = document.getElementById("ripple");
const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
const actionButtons = document.querySelectorAll("[data-action]");
setupSmoothScroll(window, reducedMotion);
const labels = {
  following: "Following your mouse",
  frozen: "Shared cursor frozen",
  hidden: "Shared cursor hidden",
};
const captions = {
  freeze:
    "Frozen. Move your mouse inside the window. The shared cursor stays put.",
  hide: "Hidden. Your mouse still works. Press Resume to bring the shared cursor back.",
  resume: "Following again. The shared cursor is back with your mouse.",
  drop: "Dropped at your mouse position. The shared cursor stays here.",
};
let animationTimer;
let paintedMode;

function paint() {
  privatePointer.style.left = `clamp(10px, ${cue.real.x}%, calc(100% - 10px))`;
  privatePointer.style.top = `clamp(10px, ${cue.real.y}%, calc(100% - 10px))`;
  audience.style.left = `clamp(10px, ${cue.audience.x}%, calc(100% - 10px))`;
  audience.style.top = `clamp(10px, ${cue.audience.y}%, calc(100% - 10px))`;
  audience.classList.toggle("label-left", cue.audience.x > 75);
  privatePointer.classList.toggle("label-left", cue.real.x > 75);
  audience.classList.toggle("label-top", cue.audience.y > 75);
  privatePointer.classList.toggle("label-top", cue.real.y > 75);
  if (paintedMode === cue.mode) return;
  paintedMode = cue.mode;
  audience.style.visibility = cue.visible ? "visible" : "hidden";
  workspace.classList.toggle("independent", cue.mode !== "following");
  modeLabel.textContent = labels[cue.mode];
  actionButtons.forEach((button) => {
    const selected =
      (button.dataset.action === "freeze" && cue.mode === "frozen") ||
      (button.dataset.action === "hide" && cue.mode === "hidden") ||
      (button.dataset.action === "resume" && cue.mode === "following");
    button.setAttribute("aria-pressed", String(selected));
  });
}

function act(action) {
  clearTimeout(animationTimer);
  if (action === "freeze" && audience.classList.contains("smooth")) {
    const pointerBounds = audience.getBoundingClientRect();
    const bounds = workspace.getBoundingClientRect();
    cue.audience = {
      x:
        ((pointerBounds.left + pointerBounds.width / 2 - bounds.left) /
          bounds.width) *
        100,
      y:
        ((pointerBounds.top + pointerBounds.height / 2 - bounds.top) /
          bounds.height) *
        100,
    };
  }
  audience.classList.toggle(
    "smooth",
    (action === "resume" || action === "drop") && !reducedMotion.matches,
  );
  cue.act(action);
  paint();
  caption.textContent =
    action === "hide" && cue.visible
      ? "Revealed. Your shared cursor returns to its previous mode."
      : captions[action];
  animationTimer = setTimeout(() => audience.classList.remove("smooth"), 280);
}

workspace.addEventListener("pointermove", (event) => {
  const bounds = workspace.getBoundingClientRect();
  cue.move(
    ((event.clientX - bounds.left) / bounds.width) * 100,
    ((event.clientY - bounds.top) / bounds.height) * 100,
  );
  paint();
});
workspace.addEventListener("pointerdown", (event) => {
  const bounds = workspace.getBoundingClientRect();
  cue.move(
    ((event.clientX - bounds.left) / bounds.width) * 100,
    ((event.clientY - bounds.top) / bounds.height) * 100,
  );
  workspace.focus({ preventScroll: true });
  paint();
  if (!cue.visible) return;
  ripple.style.left = audience.style.left;
  ripple.style.top = audience.style.top;
  ripple.classList.remove("pop");
  void ripple.offsetWidth;
  ripple.classList.add("pop");
});
actionButtons.forEach((button) =>
  button.addEventListener("click", () => act(button.dataset.action)),
);
document.getElementById("reset-demo").addEventListener("click", () => {
  clearTimeout(animationTimer);
  audience.classList.remove("smooth");
  cue.act("resume");
  cue.move(63, 53);
  paint();
  caption.textContent = "Move your mouse inside the preview. Try Freeze.";
});
document.querySelector(".demo").addEventListener("keydown", (event) => {
  if (event.ctrlKey || event.altKey || event.metaKey || event.shiftKey) return;
  const actions = { f: "freeze", h: "hide", r: "resume", d: "drop" };
  const action = actions[event.key.toLowerCase()];
  if (action) {
    event.preventDefault();
    act(action);
  } else if (event.target === workspace && event.key.startsWith("Arrow")) {
    event.preventDefault();
    cue.move(
      cue.real.x +
        (event.key === "ArrowRight" ? 3 : event.key === "ArrowLeft" ? -3 : 0),
      cue.real.y +
        (event.key === "ArrowDown" ? 3 : event.key === "ArrowUp" ? -3 : 0),
    );
    paint();
  }
});

paint();
const header = document.getElementById("header");
let headerScrolled;
function updateHeader() {
  const scrolled = window.scrollY > 48;
  if (scrolled !== headerScrolled) {
    header.classList.toggle("scrolled", scrolled);
    headerScrolled = scrolled;
  }
}
window.addEventListener("scroll", updateHeader, { passive: true });
updateHeader();
