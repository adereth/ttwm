You’ve basically got three “layers” you can test, and you’ll probably want at least two of them:

## 1) Test most of your WM *without* X at all (fast, deterministic)

**Idea:** push as much logic as possible behind a thin “X backend” interface.

What to test here:

* **Layout algorithms** (tiling, gaps, monocle, floating placement, focus rules)
* **State machines** (workspace switching, focus cycling, urgency, rules/matching)
* **Policy** (EWMH decisions, keybinding resolution, command parsing, config reload)
* **Geometry invariants** (no overlap when you claim you don’t overlap, bounds obeyed, etc.)

How:

* Classic unit tests + **property-based tests** (generate random trees of containers/windows, random sequences of “map/unmap/focus/configure” events, assert invariants).
* Run under **ASan/UBSan/TSan** in CI to catch memory races / UB early.

This layer is where you get reliable coverage without fighting timing.

## 2) Integration tests against a headless/nested X server (real X semantics, still automatable)

Run your WM under:

* **Xvfb** (headless virtual framebuffer) — best for CI.
* **Xephyr** (nested X server in a window) — great for local dev because you can *see* what’s happening.

Test harness flow (typical):

1. Start `Xvfb :99 -screen 0 1280x800x24` (or Xephyr)
2. `DISPLAY=:99` start your WM
3. Launch test clients (`xterm`, tiny custom Xlib/XCB client, GTK/Qt smoke apps)
4. Drive input via **XTEST** (or tools like `xdotool`)
5. Assert via:

   * `xprop`, `xwininfo`, `wmctrl` (properties, stacking order, geometry)
   * Direct XCB queries (more robust than scraping CLI output)

Key tricks to keep it non-flaky:

* Prefer **waiting on events** (MapNotify/ConfigureNotify/PropertyNotify) over `sleep(0.1)`.
* Add a “test sync” primitive: do an X11 **round-trip** (e.g., `xcb_get_input_focus` or `xcb_query_tree` + `xcb_flush`) to force ordering.

## 3) End-to-end “black box” tests (hardest, but catches the real bugs)

This is: “start the WM, run real apps, simulate a user, verify screen/result.”

Options:

* **Input driving**

  * XTEST (directly)
  * `xdotool` (simple, but adds another dependency layer)
* **Visual assertions**

  * Screenshot from Xvfb (`xwd` → convert) and do pixel diffs (ImageMagick `compare`) or perceptual diffs (less brittle).
  * Or avoid pixels and assert via X properties/geometry whenever possible.

Good E2E targets:

* Focus/raise behavior under rapid window creation/destruction
* Multi-monitor (XRandR) behaviors: add/remove outputs, move workspaces, pointer warps
* Edge cases: override-redirect windows, fullscreen, dialogs/transients, urgency, grabs

## 4) Spec compliance testing (ICCCM/EWMH)

There isn’t one blessed “official” conformance harness like you’d get with a browser engine, but you can still test systematically:

* Build a **small suite of purpose-built clients** that exercise:

  * Transients (`WM_TRANSIENT_FOR`), `WM_HINTS`, `WM_PROTOCOLS` (delete window), `WM_STATE`
  * EWMH: `_NET_WM_STATE`, `_NET_ACTIVE_WINDOW`, `_NET_CLIENT_LIST`, `_NET_WM_DESKTOP`, `_NET_SUPPORTED`
* Use assertions by reading properties off root and client windows.

This tends to catch “works with xterm, breaks with Chromium” class issues early.

## 5) Protocol/event fuzzing (surprisingly effective for WMs)

WMs are basically event processors. You can fuzz:

* Sequences of X events (map/unmap/configure/property changes) against your internal state machine (layer 1).
* If you have an IPC/command interface, fuzz that too (parsing + semantics).

Goal: crash-free + invariant-preserving over long random runs.

## 6) “Spy” testing: record/replay and tracing

Useful when debugging regressions:

* Trace X protocol with tools like **xtrace/xscope** (record what requests/events happened).
* Not usually “assertion tests” by themselves, but great for:

  * reproducing weird client interactions
  * regression tests: “this exact sequence used to crash us”

## A practical setup that works well in CI

* Unit/property tests: `cargo test` / `ctest` / etc. (no X)
* Integration/E2E:

  * `Xvfb` + start WM + run small clients
  * Drive input via XTEST
  * Assertions via XCB queries + occasional screenshots on failure (dump artifacts)

---

If you tell me what stack you’re using (Xlib vs XCB, language, and whether you’re ICCCM/EWMH-minimalist or “i3-ish complete”), I can sketch a concrete test harness layout (process orchestration, event waiting strategy, and a minimal “test client” you can reuse for a lot of cases).
