// Island open/close FSM — port of IslandStateMachine.swift.
// No DOM, no Tauri: it only reports transitions.

export type FsmState = "hidden" | "petit" | "home" | "coucou" | "ball";

export class IslandStateMachine {
  state: FsmState = "hidden";

  onTransition: ((from: FsmState, to: FsmState) => void) | null = null;

  /**
   * True while no coding agent is mid-session. Decides what the island does when
   * it has been left alone in the notch: with nothing running it floats out as a
   * ball instead of vanishing; with a session running it still tucks away.
   */
  isIdle: () => boolean = () => false;
  /** ball → notch delay once nobody is touching it, seconds. */
  ballReturnDelay = 20;
  /**
   * Set when the ball has flown back to the notch: from then on an idle notch
   * stays put (it neither hides nor pops out again) until something happens.
   */
  private restInNotch = false;

  /** home → petit delay, seconds. */
  homeToPetitDelay = 15;
  /** petit → hidden delay, seconds. */
  petitToHiddenDelay = 60;
  /** coucou → petit once the greeting animation ends (no hover). */
  greetAutoCollapseDelay = 0.6;
  /** coucou → petit while the mouse hovers the greeting. */
  greetHoverCollapseDelay = 10;
  /** An alert waiting for an answer stays open, even when the mouse leaves. */
  pinned = false;

  private petitHide: number | null = null;
  private homeCollapse: number | null = null;
  private greetCollapse: number | null = null;
  private ballReturn: number | null = null;

  // ── Inputs ──────────────────────────────────────────────────────────────────

  launch() {
    this.cancelTimers();
    this.transition("coucou");
  }

  mouseEntered() {
    switch (this.state) {
      case "hidden":
        this.cancelTimers();
        this.transition("petit");
        break;
      case "petit":
        this.clear("petitHide");
        break;
      case "home":
        this.clear("homeCollapse");
        break;
      case "coucou":
        this.scheduleGreetCollapse(this.greetHoverCollapseDelay);
        break;
      case "ball":
        // Someone is about to grab it: it must not fly off under their hand.
        this.clear("ballReturn");
        break;
    }
  }

  mouseLeft() {
    switch (this.state) {
      case "hidden":
        break;
      case "petit":
        this.schedulePetitHide();
        break;
      case "home":
        this.scheduleHomeCollapse();
        break;
      case "coucou":
        this.clear("greetCollapse");
        this.transition("petit");
        break;
      case "ball":
        this.scheduleBallReturn();
        break;
    }
  }

  click() {
    if (this.state !== "petit") return;
    this.cancelTimers();
    this.transition("home");
  }

  /** Greeting animation finished (T.end). Doesn't override a running hover timer. */
  greetComplete() {
    if (this.state !== "coucou") return;
    if (this.greetCollapse == null) this.scheduleGreetCollapse(this.greetAutoCollapseDelay);
  }

  /** Non-alert work event: show compact from hidden. */
  reveal() {
    // Work started again: the ball has nothing left to wait for, back to the notch.
    if (this.state !== "hidden" && this.state !== "ball") return;
    this.cancelTimers();
    this.restInNotch = false;
    this.transition("petit");
    this.schedulePetitHide();
  }

  /** Alert or explicit request: open straight to expanded. */
  forceHome() {
    this.cancelTimers();
    this.restInNotch = false;
    this.transition("home");
  }

  /// Explicit close (OK button, Escape, click outside, an alert being answered).
  forcePetit() {
    this.cancelTimers();
    this.restInNotch = false;
    this.transition("petit");
  }

  /** Nothing is running: the notch lets Mochi float out as a ball. */
  ballify() {
    if (this.state === "ball" || this.state === "coucou") return;
    this.cancelTimers();
    this.transition("ball");
    this.scheduleBallReturn();
  }

  /**
   * The ball was picked up and put down. It stays out while it is under the
   * pointer and goes back to counting down once the pointer is gone.
   */
  ballTouched(pointerOver: boolean) {
    if (this.state !== "ball") return;
    this.clear("ballReturn");
    if (!pointerOver) this.scheduleBallReturn();
  }

  forceHidden() {
    this.cancelTimers();
    this.transition("hidden");
  }

  // ── Timers ──────────────────────────────────────────────────────────────────

  private schedulePetitHide() {
    this.clear("petitHide");
    // A notch that already had its turn as a ball just stays where it is.
    if (this.restInNotch && this.isIdle()) return;
    this.petitHide = window.setTimeout(() => {
      this.petitHide = null;
      if (this.state !== "petit") return;
      // Left alone with nothing running: float out instead of disappearing.
      const next = this.isIdle() ? "ball" : "hidden";
      this.transition(next);
      if (next === "ball") this.scheduleBallReturn();
    }, this.petitToHiddenDelay * 1000);
  }

  private scheduleBallReturn() {
    this.clear("ballReturn");
    this.ballReturn = window.setTimeout(() => {
      this.ballReturn = null;
      if (this.state !== "ball") return;
      this.restInNotch = true;
      this.transition("petit");
    }, this.ballReturnDelay * 1000);
  }

  private scheduleHomeCollapse() {
    this.clear("homeCollapse");
    if (this.pinned) return;
    this.homeCollapse = window.setTimeout(() => {
      this.homeCollapse = null;
      if (this.state === "home") this.transition("petit");
    }, this.homeToPetitDelay * 1000);
  }

  private scheduleGreetCollapse(delay: number) {
    this.clear("greetCollapse");
    this.greetCollapse = window.setTimeout(() => {
      this.greetCollapse = null;
      if (this.state === "coucou") this.transition("petit");
    }, delay * 1000);
  }

  private clear(which: "petitHide" | "homeCollapse" | "greetCollapse" | "ballReturn") {
    const id = this[which];
    if (id != null) window.clearTimeout(id);
    this[which] = null;
  }

  cancelTimers() {
    this.clear("petitHide");
    this.clear("homeCollapse");
    this.clear("greetCollapse");
    this.clear("ballReturn");
  }

  private transition(next: FsmState) {
    if (next === this.state) return;
    const from = this.state;
    this.state = next;
    this.onTransition?.(from, next);
  }
}
