export class CueDemo {
  constructor() {
    this.real = { x: 63, y: 53 };
    this.audience = { ...this.real };
    this.mode = "following";
    this.previous = "following";
  }
  get visible() {
    return this.mode !== "hidden";
  }
  move(x, y) {
    this.real = {
      x: Math.max(0, Math.min(100, x)),
      y: Math.max(0, Math.min(100, y)),
    };
    if (this.mode === "following") this.audience = { ...this.real };
  }
  act(action) {
    if (action === "hide") {
      if (this.mode === "hidden") {
        this.mode = this.previous;
        if (this.mode === "following") this.audience = { ...this.real };
      } else {
        this.previous = this.mode;
        this.mode = "hidden";
      }
    } else if (action === "freeze") {
      this.mode = "frozen";
    } else if (action === "resume" || action === "drop") {
      this.mode = action === "resume" ? "following" : "frozen";
      this.audience = { ...this.real };
    }
  }
}
