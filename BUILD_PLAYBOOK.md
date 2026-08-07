# Playbook: browser-native scientific fitting tools (Rust/WASM + TS)

**Purpose of this document.** This is a reference for building another browser-based
scientific analysis tool with the same architecture and UX as `hetdma-web`
(a Rust/WASM port of an R battery-degradation-mode-analysis package), but for a
different backend domain (ICI data analysis). Hand this to the agent alongside the
actual spec for the new app. **Everything about the *math/domain* in this document is
hetdma-specific and must come from the new spec, not from here.** What should transfer
is the *architecture*, the *UX patterns*, the *build/deploy pattern*, and the *specific
gotchas* listed below - all backend-agnostic.

---

## 1. Reference architecture

```
repo/
  Cargo.toml            # workspace: members = ["core", "wasm"]
  core/                 # pure Rust numeric library - NO wasm-bindgen, NO DOM.
    src/*.rs             # compiles and unit-tests natively (fast iteration, real debugging)
    tests/*.rs           # includes the golden-file test (see §3) - this is the project's
                          # correctness backbone, not an afterthought
  wasm/                  # thin wasm-bindgen shim over `core` - NO numeric logic of its own
    src/lib.rs            # #[wasm_bindgen] fns, JSON in/out via serde_json, nothing else
  web/                    # Vite + vanilla TypeScript front end, no framework
    src/
      state.ts             # AppState + a ~40-line hand-rolled observable store
      wasm/                 # main-thread WASM instance (for fast/live evaluations)
        pkg/                 # wasm-pack output - lives under src/, NOT public/ (see §6)
      worker/               # Web Worker pool for the parallel/expensive computation
      panels/               # one file per left-column UI section
      plots/                # charting wrapper
  netlify.toml            # builds Rust+wasm-pack+Vite in one CI step (see §7)
```

**Why this split:**
- `core` has zero DOM/wasm-bindgen dependencies, so `cargo test` runs it natively - fast
  compile, fast test cycles, real debuggers, no browser needed to validate the actual
  math. This is where nearly all the correctness work happens.
- `wasm` is deliberately thin: JSON parse in, call into `core`, JSON serialize out.
  Keeping numeric logic out of this crate means the WASM boundary never needs
  re-validating when the math changes - only when the *API shape* changes.
- `web` is plain TypeScript with a tiny hand-rolled store, not React/Vue/Svelte. This
  was a deliberate choice because the chosen chart library (uPlot) is an imperative
  canvas API that fights virtual-DOM diffing, and the app is a single screen with no
  routing - a framework's main benefit (component composition at scale) didn't apply.
  Reconsider this choice if the new app's UI is more complex/multi-page.

---

## 2. Recommended build order (milestones)

Build in this order; each milestone should be *verified working* (tests green, and for
UI milestones, actually driven in a real browser with real data) before starting the
next one:

1. **Toolchain + scaffolding.** rustup, `wasm32-unknown-unknown` target, `wasm-pack`,
   Node via nvm (check the system Node version first - it's often too old). Empty
   Cargo workspace + empty Vite project, confirm `cargo test`, `wasm-pack build`, and
   `npm run dev` all work before writing any real code.
2. **Golden-file fidelity gate (blocking).** If porting from a reference implementation
   (R, Python, MATLAB, a spreadsheet, whatever), this is the non-negotiable first
   deliverable - see §3. No UI work starts until this passes.
3. **WASM boundary + data import UI.** Expose parse/prepare functions, build the file
   import panel + raw data viewer. Verify by loading real files in a real browser and
   spot-checking parsed output against the reference implementation's own parsing.
4. **Visualization + live preview.** Charts plus a fast, low-latency "evaluate at these
   parameter values" preview path independent of the full analysis/fit (see §5).
5. **The expensive computation** (global search fit, batch analysis, whatever the
   domain's heavy lifting is) via a Web Worker pool, with progress reporting.
6. **Results display + post-computation visualization polish.**
7. **Polish**: axis rescaling, tooltips, export, save/load config.

At each milestone, run the full test suite and a real browser check before moving on -
don't let visual/behavioral regressions accumulate silently.

---

## 3. Golden-file fidelity testing (if porting a reference implementation)

If there's an existing trusted implementation (this project ported an R package), the
single highest-leverage thing to do first is:

1. Write a small script *in the reference implementation's own language* that exports:
   - Parsed/cleaned input fixtures (post any unit conversion, filtering, resampling the
     reference implementation does), so the port's *parsing* can be checked
     independently of its *math*.
   - For a handful of representative parameter/config vectors spanning the input space:
     the reference implementation's own computed outputs (objective values, derived
     curves, whatever the domain's core output is), with enough numeric precision to
     actually verify against (watch for JSON export libraries that silently truncate
     float precision, or that serialize `NA`/missing values as a string instead of
     `null` - both bit us here).
2. In the ported language's test suite, load those fixtures and assert the port
   reproduces them to a tight, explicit tolerance (relative tolerance on scalar values,
   absolute tolerance on curves). Gate all further work on this test passing.
3. Additionally isolate and test the highest-risk individual steps (interpolation/
   inversion, any sign/orientation convention, any smoothing window) with small,
   hand-checkable micro-fixtures, not just the end-to-end golden case - when the
   end-to-end test fails, you want to know *which* step broke.

This is worth the setup cost even under time pressure: it turns "does the port look
about right" into a hard pass/fail check, and it caught several real bugs early in this
project that would otherwise have surfaced as confusing behavior deep into UI work.

---

## 4. UX patterns to replicate

The user explicitly wants the *layout, file handling, and analysis workflow feel*
carried over. Concretely, what worked well here:

**Desktop-only three-column layout** (no mobile support, no responsive breakpoints):
left column = file import + parameter/config controls + "run" button; center column
(majority of the screen) = plots + results; right column = raw data viewer for the
imported files.

**File import panel:**
- Drag-and-drop zones with a click-to-browse fallback, one per required input file.
- Tolerant parsing: sniff delimiter (tab vs comma) rather than assuming one; look for
  named header columns first, fall back to column position.
- Domain sanity checks with a visible warning + an explicit override toggle, rather
  than silently guessing or hard-failing. (Here: checking curve orientation/sign
  convention against what the model physically requires, not just against "does it
  look like the other file.")
- Auto-populate derivable config values from the data itself where possible (here: an
  electrode-area estimate from a capacity ratio), shown with a clear "this was
  estimated, verify it" note that disappears the moment the user edits the field
  manually. This measurably improves first-run experience - the user sees a sensible
  starting result immediately instead of a blank screen waiting for manual config.

**Parameter/config panel:** for any per-parameter specification (if the new domain has
fittable parameters), the fixed/free-with-bounds/free-with-prior pattern worked well
and is worth reusing if applicable - but this is a hetdma-specific need; adapt to
whatever the ICI domain's actual configuration surface is.

**Live preview, decoupled from the full computation:** a fast "evaluate right now at
these exact settings" path, separate from whatever the expensive full
analysis/optimization is, wired directly to the input controls (sliders/fields) with
`requestAnimationFrame`-throttling and a low-resolution-while-dragging /
full-resolution-on-release split. This used a **second, always-resident WASM instance
kept on the main thread**, deliberately separate from the worker pool used for the
expensive computation, so the live preview stays responsive even while a full run is in
progress. This pattern is very likely worth reusing for ICI data analysis too if
there's any notion of "preview the effect of a setting before committing to a full
run."

**Charting:** uPlot (not Plotly/Chart.js/etc.) specifically for redraw performance
under a live-preview slider being dragged continuously - this is an imperative,
low-level canvas API, more code to wire up than a batteries-included library, but
dramatically faster for this use case. Multiple panels sharing a synced cursor.
Explicit Y-axis rescale controls (see the uPlot gotcha in §6).

**Expensive computation via Web Worker pool:** one WASM instance per
`navigator.hardwareConcurrency` core, pull-based work dispatch (idle workers request
the next unit of work, rather than a static split) so one slow unit of work doesn't
stall the whole pool, live progress reporting (completed/total, best-so-far metric,
ETA). Message-passing only - **no `SharedArrayBuffer`** (see §7 for why this matters
for deployment).

**Results + export:** a results table below the plots reporting derived,
human-interpretable summary quantities rather than raw internal parameters where the
two differ (here: a distribution's interpretable percentile spread rather than its raw
internal shape parameter). JSON export of results, CSV export of output curves,
save/load of the input configuration as JSON.

---

## 5. Numerical safety pattern (do this regardless of domain)

**Never let `NaN`/`Infinity` reach the JS boundary.** `serde_json` silently serializes
Rust `f64::NAN`/`f64::INFINITY` as JSON `null`. Naive JS/TS code that expects a number
(e.g. `value.toPrecision(5)`) then throws a confusing `Cannot read properties of null`
error, often minutes into a long-running computation, which is a miserable thing to
debug from the JS side alone. The fix: pick one finite sentinel value for
"invalid/degenerate result" (this project used `1e100`) and sanitize to it at *every*
point a computation could produce a non-finite value - inside the objective/cost
function, after reading back an optimizer's result, anywhere a division or an external
solver library is involved. Add the check even where you're confident it can't happen;
optimizer internals and edge-case inputs are exactly where "can't happen" turns out to
be wrong. Belt-and-suspenders on the JS side too: if a whole computation's "best result"
is stuck at the sentinel, surface a clear error message instead of silently displaying
a nonsense result.

---

## 6. Concrete gotchas hit in this project (read before you rediscover them)

- **`wasm-pack build <crate-path> --out-dir <dir>` resolves `--out-dir` relative to
  `<crate-path>`, not the invoking working directory.** Running
  `wasm-pack build wasm --out-dir web/src/wasm/pkg` from the repo root actually writes
  to `wasm/web/src/wasm/pkg`. This broke the Netlify build after working fine locally
  (where the working npm script happened to `cd` into the right place first). If your
  build command runs from the repo root, use `../web/src/wasm/pkg` instead.
- **Vite refuses to statically or dynamically import JS from `public/`.** wasm-pack
  output must live under `src/` as a normal ES module (e.g. `src/wasm/pkg/`), imported
  the ordinary way. Don't follow older wasm+Vite guides that put the wasm-pack output
  in `public/` - it will build fine locally in some configurations and then fail with
  `Cannot find module` in others.
- **uPlot's `setData(data, resetScales)`: `resetScales=false` disables auto-fitting
  entirely, not just for manually-set scales.** Naively setting this to `false` "so a
  custom Y-range survives a data refresh" instead makes the chart never auto-fit at
  all, including on first load (blank chart until a user manually touches the range
  controls). The correct pattern: always call `setData(data, true)` (let it auto-fit),
  then immediately re-apply any stored custom range via `setScale('y', {min, max})` on
  top, every update. This gives "auto-refreshes normally, but a custom range always
  wins" rather than "a custom range disables refreshing."
- **Debug vs release Rust performance differs by 10x+** for numeric-heavy code with
  many small function calls (interpolation, sorting, etc.). Always benchmark/profile
  performance-sensitive paths with `cargo test --release` or the actual `wasm-pack
  build --release` output, not a plain `cargo test`, before concluding something is
  pathologically slow.
- **The Node version bundled with your build/dev environment may be too old for current
  Vite** (this project's system Node was v14; Vite needs 18+). Use `nvm`, and set a
  default alias so new shells pick up the right version automatically. Pin the Node
  version explicitly in the Netlify config too - don't rely on its default.
- **Sign/orientation conventions are a recurring source of subtle, hard-to-notice
  bugs**, especially when a forward model works in one internal convention (e.g.
  charge-sense) and flips to another for display/comparison (e.g. discharge-sense) at
  the very end. When overlaying *auxiliary* reference data on a chart whose axis is in
  the *flipped* convention, the auxiliary data usually needs the same flip applied -
  it's easy to overlay it raw and get something that looks plausible but is backwards.
  When in doubt, derive the correct convention from the underlying equations on paper,
  not from "does it look about right" - and where practical, add a small automated
  regression test that cross-checks a derived display quantity against an
  independently-computed expectation (e.g. "these two displayed component curves
  should sum back to the primary curve").

---

## 7. Deployment (Netlify)

This project deploys as a fully static site with **no backend, no SharedArrayBuffer,
no COOP/COEP headers** - the Web Worker pool is deliberately message-passing only,
which is precisely what makes "any static host, zero server config" possible. Keep
that constraint in the new app unless there's a specific reason to need
`SharedArrayBuffer` (there usually isn't, for this class of tool).

`netlify.toml` pattern that worked (adjust paths for the new repo layout, and mind the
`wasm-pack --out-dir` gotcha above):

```toml
[build]
  command = "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable && source $HOME/.cargo/env && rustup target add wasm32-unknown-unknown && curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh && wasm-pack build wasm --target web --release --out-dir ../web/src/wasm/pkg && cd web && npm ci && npm run build"
  publish = "web/dist"

[build.environment]
  NODE_VERSION = "20"
```

Rust isn't preinstalled on Netlify's build image, so every build pays a "install
rustup + wasm-pack" cost (a couple of minutes). Fine for occasional deploys; if that
becomes annoying, the alternative is committing the prebuilt `wasm-pack` output and
skipping Rust in CI entirely - a real tradeoff (drift risk vs build time), not a clear
win either way.

**Before considering deployment done: build and serve the actual production bundle
locally** (`npm run build` + `vite preview`, or equivalent) and drive it in a real
browser, not just the dev server. Worker bundling and WASM loading can behave
differently between dev and a production build.

---

## 8. Process notes (how this project was actually run)

- Milestones were built and *verified end-to-end in a real browser with real data*
  after each one, not just unit-tested - for a UI-heavy tool, "the tests pass" and "the
  feature actually works when you use it" are different claims, and only the second one
  matters to the user.
- Where a fix's correctness could be checked *numerically* rather than just visually
  (e.g., "does this derived quantity match an independent calculation to N decimal
  places"), that was strongly preferred over eyeballing a screenshot - screenshots are
  good for confirming something is visible/styled correctly, not for confirming a
  number is right.
- Git commits happened only when explicitly requested, kept small and scoped to one
  logical change, with a `.gitignore` curated up front to keep build artifacts and any
  proprietary/reference data (this project excluded the original R package and its
  real experimental data fixtures) out of the repo - worth confirming with the user
  early what should and shouldn't be committed, since "obviously exclude X" is often
  genuinely ambiguous (e.g., excluding fixture data also disables the golden-file tests
  for anyone else who clones the repo - a real tradeoff worth surfacing, not deciding
  silently).
- When something was genuinely ambiguous and consequential (which plotting library,
  whether to commit reference data, how to handle a missing config value), the choice
  was surfaced as a short, concrete question rather than guessed - but implementation
  details with a clear reasonable default were just decided and noted, not asked about.
