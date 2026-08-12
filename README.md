# ICI Studio

ICI Studio is a browser-native tool for analysing **intermittent current
interruption (ICI)** measurements from battery cyclers. You drop in a raw
cycler export, map its columns, and the app segments the current profile
into active/rest periods, fits a resistance model to every rest, smooths
the results, and derives incremental-capacity-style quantities — all
displayed as interactive plots and a sortable results table, with export
to TSV, PNG, and JSON.

It is a from-scratch Rust/WebAssembly + TypeScript port of an R analysis
script, built to run entirely client-side: **nothing you import is ever
uploaded anywhere.** Parsing and all numerical analysis happen in a Web
Worker running compiled Rust (via WebAssembly) inside your own browser
tab. The app is a static site with no backend — see [Privacy and data
handling](#privacy-and-data-handling) below.

## What is an ICI measurement?

An intermittent current interruption experiment applies a current pulse
to a cell (charge or discharge), then rests it (zero current) for a
period before the next pulse. During each rest, the voltage relaxes
toward its equilibrium value. Fitting that relaxation — classically
`E(t) ≈ E₀ − ΔI·R − ΔI·k·√t` — separates an instantaneous ohmic
resistance `R` from a diffusion-like time-dependent term `k`. Repeating
this across a whole charge or discharge sweep gives `R(Q)` and `k(Q)`
curves that are far more informative than a single pulse test.

This tool automates that whole pipeline: finding every rest period,
fitting the relaxation model to each one, smoothing the resulting `R`
and `k` series against state of charge, and computing their derivatives.

---

## Quick start

1. Open the app and drag a data file onto the **Import** panel (or use
   "Browse files…"). See [What files parse correctly](#what-files-parse-correctly-and-what-may-not)
   below for what's supported.
2. Check the **Parse card** — it reports what the parser auto-detected
   (encoding, delimiter, decimal separator, header, preamble lines). Fix
   anything wrong with a manual override.
3. In **Column mapping**, confirm (or correct) which columns map to
   time, cycle number, current, voltage, and charge, and their units.
   Common column names are auto-detected.
4. Set the **State threshold** in the Stage A panel if the app's
   suggested value looks better than the default (see
   [State classification](#state-classification) below), then click
   **Fit resistances**. **Smooth & derive** (Stage B) runs automatically
   afterward.
5. Explore Plots 1–4, the results table, and summary statistics. Add
   custom plots, tune QC flags, and export TSV/config/run-report/PNGs
   as needed.

---

## The analysis pipeline

### 1. Parsing

A generic, instrument-agnostic delimited-text parser reads the file:
detects encoding, delimiter, decimal separator, an optional preamble,
and an optional header row, then classifies each column as numeric or
text. `.mf4` files instead go through a dedicated MDF4 binary reader that
decodes every channel group into the same column table, so everything
downstream (mapping, segmentation, fitting) is unaware of which importer
ran. See [What files parse correctly](#what-files-parse-correctly-and-what-may-not).

### 2. Column mapping

You tell the app which columns hold **time**, **cycle number**,
**current**, **voltage**, and **charge**, and their units (e.g. `mA` vs
`A`, `mAh` vs `Ah`). Everything downstream works in SI-ish internal
units (seconds, volts, amps, amp-hours) — the conversion happens once,
here. Non-numeric columns can additionally be marked as **grouping
columns** (e.g. a sample ID or temperature label in a multi-cell export)
so the whole pipeline runs independently per group.

### 3. Segmentation and state classification

Each row is classified as **charge**, **discharge**, or **rest** by
comparing its current against a **state threshold** (default: any
non-zero current is "active"; raise it if your instrument reports a
small non-zero "resting" current). The app will suggest a threshold
automatically if it detects the file's true rest points are being
misclassified as active. Rows are grouped into contiguous **rest
periods**, each carrying the active run that preceded it.

**Q anchoring** controls where charge (`Q`) is zeroed for each
charge/discharge half-cycle — at the start (default, matching the
original R implementation) or the end. This is a display/fitting
reference choice, not a physical assumption.

**ICI cycle detection** (on by default; tuned in the Stage A panel)
additionally excludes rests that aren't part of the actual ICI cycle
*before* Q anchoring or fitting ever sees them: a rest longer than a
configurable duration (default 5 minutes — e.g. an OCV rest between
cycles) is dropped outright, and any run of fewer than a configurable
number of consecutive short rests within one cycle number (default 20 —
e.g. a DCIR leg's own handful of rests) is dropped as a group. This
keeps a heterogeneous file's non-ICI sections from corrupting Q
anchoring for the real interruptions nearby, or cluttering the
regression diagnostic. Excluded rows aren't hidden — they're shaded red
on Plot 1, and the count dropped is reported in the run log.

### 4. Stage A — resistance fitting

For every rest period, the app fits `E(t) ≈ E₀ − ΔI·R − ΔI·k·√t` by
linear regression over a configurable time window (`t_min`–`t_max`,
default 2–14 s after the interruption). Key parameters:

- **State threshold** — see above.
- **Voltage interpolation / current averaging windows** — how the
  pre-interruption voltage and current baselines are estimated.
- **Edge points** — how many points at each end of the fit window are
  used for an edge-curvature/imbalance diagnostic (used by QC, below).
- **Drop unrested reversals** (advanced) — discard a charge/discharge
  direction reversal that has no rest before it (can't be fit).
- **Legacy compatibility** (advanced) — matches a specific quirk of the
  original R script's current-averaging behaviour; leave off unless you
  need bit-for-bit parity with older R-generated results.

An **"Estimate optimal window"** tool (in the Regression window panel)
can help pick `t_min`/`t_max`: it samples a spread of rests across the
file, scores a grid of candidate windows by fit quality (mean/median
adjusted R², point count, edge diagnostics), and lets you apply the
best-ranked one with one click.

`R` and `k` are physically always positive; a negative fit is flagged as
non-physical (and can optionally be excluded from smoothing — see QC).

### 5. Stage B — smoothing and derivatives

`E₀`, `k`, and `R` are smoothed against `Q` using a monotonic
(or unconstrained) penalised B-spline, fit **independently within each
smoothing group** — by default, every combination of cycle number,
charge/discharge state, and any grouping columns you selected. You can
toggle which of those keys actually split the data, and tune each
smoother's basis dimension (`k`), penalty order (`m`), and monotonicity
direction (automatic/increasing/decreasing). `R`'s smoother can simply
**inherit** `k`'s smoother settings (the default) rather than being
configured separately.

The derivatives `dV/dQ` and `dQ/dV` are then computed from the smoothed
`E₀(Q)` curve by local polynomial differentiation (window size and
polynomial degree are configurable). **Sign convention:** these are
displayed positive on charge and negative on discharge (a presentation
choice; the underlying fit is unaffected).

### 6. Quality control

Six independent flags can highlight (and optionally exclude from
smoothing) individual rest fits:

| Flag | Trigger |
|---|---|
| Poor fit | adjusted R² below a threshold |
| Too few points | fewer fit points than a threshold |
| Edge curvature | an edge-curvature diagnostic (`edge_max_z`) above a threshold |
| Edge imbalance | an edge-balance diagnostic (`edge_mae_ratio`) above a threshold |
| Non-physical | negative `R` or `k` |
| Degenerate ΔI | the current step between active and rest is ~zero |

Each flag has its own enable toggle, threshold, and
"exclude from smoothing" switch (non-physical and degenerate-ΔI exclude
by default; the rest only highlight by default). You can also manually
force any individual rest to be included or excluded, overriding the
flags, from the results table — this survives re-running the analysis.
Flagged points render muted and excluded points render hollow on Plots
3/4; clicking a point (or a results-table row) jumps Plot 2's diagnostic
view to that exact rest.

---

## The user interface

A theme toggle (top right) cycles system → light → dark. All plots
(axes, gridlines, fit lines, shading) re-theme live, not just the
surrounding UI chrome.

**Left column** (configuration, top to bottom): Import → Parse card →
Column mapping → Regression window (+ optimal-window estimator) → Stage
A (resistance fitting) → Quality control → Stage B (smoothing &
derivatives) → Visualisation (area normalisation, display options) →
Export → Run log.

**Centre column** (plots and results):

- **Plot 1 — raw time series.** Voltage (and optionally current) vs.
  time, with rest periods shaded and the currently-selected rest
  highlighted. Zooming re-decimates the underlying data so it stays
  responsive on files with tens of thousands of rows.
- **Plot 2 — regression diagnostic.** `E` vs `√(step time)` for the
  currently-selected rest, with the fit window shown as a draggable
  shaded region, the fitted line overlaid, and live `R`/`k`/adj-R²/point
  count feedback as you drag — useful for sanity-checking the window
  choice on a specific rest before committing to it globally. A rest
  navigator (◀ ▶, jump-to-box, arrow keys) steps through every rest.
- **Plot 3 — R vs. x** and **Plot 4 — k vs. x.** Scatter of every rest's
  fitted value (coloured by charge/discharge, QC-styled per the table
  above), with error bars and one smoothing line per group. The x-axis
  is selectable (Q, t, step.t, E, I, E0, s, I0, or cycle number), as is
  a display-only unit scale (×1/×1e3/×1e6) for the very small resistance
  values this analysis typically produces.
- **Additional plots.** Add any number of custom plots of any numeric
  analysis-table column (or the smoothed/derivative columns) against any
  other, as points, a line, or both, with an optional error-bar column.
  Drag to reorder.
- **Results table.** Every rest's full fit output, sortable by column,
  filterable by charge/discharge and by QC status, with per-row manual
  QC override.
- **Summary statistics.** Per smoothing group: point count, median/IQR
  of `R` and `k`, median fit quality, `R`/`k` at the start/mid/end of the
  group's Q range, and the fitted smoother's effective basis
  size/penalty/EDF.

**Right column:** a virtual-scrolled raw-data inspector over the parsed
file, with per-column summary statistics and jump-to-row.

A restore banner offers to reapply your last session's settings
(mapping, Stage A/B config, QC thresholds, etc.) the next time you drop
a file with a matching column header — nothing about the data itself is
ever stored, only the configuration (see below).

### Display-only options (Visualisation panel)

These reshape *what's shown*, never the underlying fit or export data:

- **Electrode area + "normalise to area"** — rescale `R`, `k`, `Q`, and
  the derivatives by a per-cm² electrode area.
- **Absolute Q** — display `|Q|` (useful once Q-anchoring makes charge
  and discharge cross zero).
- **Absolute dV/dQ** — display `|dV/dQ|` (its natural sign is often read
  as "always positive" by convention; `dQ/dV` always keeps its true
  sign).

### Export

- **TSV** of the full analysed table (full precision or 6 significant
  figures; empty/`NA`/`NaN` for missing values; optional unit-header
  comment), or a smaller per-rest regression table only.
- **Configuration JSON** — every setting above, downloadable and
  re-loadable via "Load config…" (column mappings that don't exist in a
  newly-loaded file are reported and skipped; everything else still
  applies).
- **Run report JSON** — the config, file provenance (name, size, SHA-256
  hash), a structured decision/warning log, key summary statistics, and
  per-stage timings, for record-keeping or programmatic follow-up.
- **PNG** export of any individual plot, at 1×/2×/3× resolution.

---

## What files parse correctly (and what may not)

The text parser is deliberately **generic and instrument-agnostic** —
there are no per-vendor code paths, and it never reads or displays
instrument metadata. It expects: an optional preamble of any length,
followed by an optional single header row, followed by delimited numeric
data. That contract works well for typical cycler exports (e.g.
Bio-Logic EC-Lab `.mpt`/`.txt` files, and similar plain-text exports from
other cyclers), but it has real, specific limits. `.mf4` files skip this
parser entirely — see the MDF4 bullet below.

**Will parse correctly:**
- Tab-, comma-, semicolon-, or pipe-delimited text (`.txt`, `.csv`,
  `.tsv`, or no extension at all — the parser doesn't look at the file
  name).
- UTF-8 or Windows-1252/Latin-1 encoded files (auto-detected; a UTF-8
  byte-order mark is stripped automatically).
- `.` or `,` as the decimal separator (auto-detected).
- Any preamble length before the data starts, and either a single header
  row or no header at all (columns are then named `col1`, `col2`, ... by
  position).
- A trailing empty column caused by a trailing delimiter (stripped
  automatically, whether it appears on the header, every data row, or
  both).
- Missing/blank cells, and the tokens `NA`, `NaN`, `Inf`, `-Inf`,
  `null`, `-` (case-insensitive) — treated as missing, not an error.
- A handful of malformed rows (wrong field count) — dropped individually
  with a reported line number, not fatal to the whole file.
- Files up to 250 MB (a warning appears above 100 MB that parsing may
  take a moment, but it still proceeds).
- **MDF4 (`.mf4`) files** — detected by extension and routed to a
  dedicated binary reader instead of the text parser above, including
  files using MDF4's compressed (`##DZ`/zlib, optionally
  transpose-encoded) or history-list (`##HL`) storage. Every channel
  group is decoded and merged into one table, matched against the group
  with the most samples by row count and by its master (time) channel's
  first/last timestamps; a group that doesn't line up is left out of the
  table, with a warning naming it, rather than silently misaligned. The
  Parse card hides the encoding/delimiter/decimal-separator/header
  overrides for these files, since none of them apply to a binary
  source.

**May not parse correctly, or need a manual override:**
- **Fixed-width or space-delimited files** — not supported; only the
  four delimiters above are tried.
- **Very short files.** Auto-detection requires at least 5 consecutive,
  same-width, fully-numeric rows to recognise where the data block
  starts. A file with fewer than 5 data rows, or a data block interrupted
  by non-numeric rows, may fail to auto-detect — use the manual "skip N
  lines" / delimiter / header overrides in the Parse card instead.
- **Single-column files** — auto-detection requires at least 2 fields
  per row unless you force the delimiter manually.
- **Preambles longer than 5,000 lines.** The sniffer only samples the
  first 5,000 lines of the file to locate the header/data boundary. An
  unusually long preamble beyond that will need a manual "skip N lines"
  override.
- **A column that's mostly text with a few numbers, or vice versa** —
  each column is classified numeric only if at least half its non-missing
  cells parse as numbers; a column that's borderline may be classified
  the "wrong" way for your intent. Non-numeric columns can still be used
  as grouping columns, but can't be mapped to time/current/voltage/etc.
- **Multi-sheet spreadsheets, `.xlsx`/`.xls` binary formats, or JSON/XML
  exports** — out of scope; export to plain delimited text first.
- **Proprietary/binary cycler formats other than MDF4** (e.g. a raw
  `.mpr` rather than the plain-text `.mpt` export) — export the
  plain-text version from your cycler's software first.
- Whatever the parser does decide, it's always visible and correctable:
  the **Parse card** shows every sniffer decision and lets you override
  encoding, delimiter, decimal separator, lines-to-skip, and
  header-present independently, with the file re-parsed live.

If a required field's mapped column turns out non-numeric, or the same
column is mapped to two fields, that's a hard block (fix it before
continuing). Non-finite values in a mapped column, or non-monotonic
time, are warnings with a suggested one-click fix, not hard blocks.

---

## Privacy and data handling

This is a fully static site — there is no backend and no server-side
processing. Once the page has loaded, everything (parsing, segmentation,
regression, smoothing, plotting, export) runs inside a Web Worker in
your own browser tab; the file you drop in is never uploaded anywhere.
Session-restore settings are kept in your browser's `localStorage` only
(again, never uploaded) and hold configuration, not data. The one
inherent exception is that the *hosting provider* serving the static
page/JS/WASM files sees the same basic access logs (IP, requested paths,
timestamps) any web host does for any page load — that's unrelated to,
and doesn't include, anything about the files you subsequently analyse.

The production deployment (`ici-studio.lacey.se`) also loads Google
Analytics for basic page-view stats (visits, referrers, rough
geography) — a page-shell script declared in `index.html`, gated to that
exact hostname so a fork, a local dev server, or a Netlify preview URL
never reports traffic against it. It has no visibility into anything the
analysis pipeline does; it only ever sees "someone loaded the page."

---

## Development

```
core/     Rust: parsing (text + MDF4), segmentation, regression, spline smoothing, derivatives
wasm/     Thin wasm-bindgen shim over core, used by the Web Worker
web/      Vite + TypeScript front end
```

`core/src/mf4/` is a trimmed, in-place-patched subset of the
[`mf4-rs`](https://github.com/dmagyar-0/mf4-rs) crate (MIT, see
`core/src/mf4/ATTRIBUTION.md`), vendored rather than depended on so it
could gain `##HL`/`##DZ` compressed-block support the upstream crate
lacks — see that directory's own module doc comment for the full
rationale.

```bash
# Rust test suite (includes golden-file validation against the R reference)
cargo test

# Build the WASM package the web app imports
wasm-pack build wasm --target web --release --out-dir ../web/src/wasm/pkg

# Web app
cd web
npm install
npm run dev      # dev server
npm run build    # production bundle (tsc + vite build)
npm run preview  # serve the production build locally
```

Deployment is a static build (`web/dist`) with no special server
configuration — see `netlify.toml` for the CI build command used on
Netlify.

## License

MIT — see [LICENSE](LICENSE).
