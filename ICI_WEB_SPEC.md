# `ici-web` — build specification

**Status:** ready for implementation hand-off.
**Read alongside:** `BUILD_PLAYBOOK.md` (architecture, build/deploy pattern, gotchas — all
backend-agnostic and still applies) and `ici_analysis.R` (the reference implementation and
the sole authority on the maths).

**Rule of precedence.** Where this spec and `ici_analysis.R` disagree on numerical
behaviour, `ici_analysis.R` wins *unless* the disagreement is listed explicitly in §14
(Deviations from R). Anything else that looks like a conflict is a spec bug — stop and ask
Matt rather than deciding silently.

---

## 1. What the app is

A browser-native, fully static, desktop-only tool for intermittent current interruption
(ICI) analysis of battery cycling data. It ports `ici_analysis.R` to Rust compiled to WASM,
wraps it in a Vite + TypeScript front end, and deploys to Netlify with no backend.

Same shape as `hetdma-web`: left column for import and configuration, main area for a
two-column grid of plots plus summary statistics.

### 1.1 The physics, in one paragraph

During an ICI experiment the cell is pulsed with a constant current and then rested
repeatedly. Immediately after each interruption the voltage relaxes; over a short window the
relaxation is approximately linear in √t:

```
E(t) = E0 + s·√t
```

Fitting that line over a rest period gives the intercept `E0` (voltage extrapolated to the
instant of interruption) and slope `s`. With `ΔI = I − I0` (the current before the
interruption minus the current during the rest, normally ≈ the pulse current), the
resistance and the diffusion-resistance coefficient are

```
R = (E − E0) / ΔI          [Ω]
k = −s / ΔI                [Ω s^-1/2]
```

`E` here is the voltage interpolated at the *end* of the preceding active step. Errors
propagate from the regression standard errors:
`R_err = E0_err/|ΔI|`, `k_err = s_err/|ΔI|`.

Everything else in the pipeline exists to (a) find the rests and active steps reliably,
(b) get `E`, `I`, `I0` and `Q` right for each interruption, and (c) smooth `R`, `k` and `E0`
against charge and take derivatives.

---

## 2. Architecture

Follow `BUILD_PLAYBOOK.md` §1 exactly, with the deviations below.

```
ici-web/
  Cargo.toml              # workspace: members = ["core", "wasm"]
  core/                   # pure Rust: parsing, segmentation, regression, splines. No wasm-bindgen.
    src/
      parse.rs            #   delimited-text sniffing + EC-Lab preamble handling
      segment.rs          #   state classification, rest indexing, step time, Q anchoring
      regress.rs          #   per-rest OLS on E ~ sqrt(step.t), diagnostics
      derive.rs           #   R, k and their errors; non-physical handling
      spline.rs           #   P-spline / SCOP-spline smoother  (highest-risk module)
      deriv.rs            #   local moving-window polynomial derivative
      window.rs           #   optimal regression-window search
      types.rs            #   config structs, result structs, serde
    tests/                #   golden-file + micro-fixture tests
  wasm/                   # thin #[wasm_bindgen] shim: JSON/ArrayBuffer in, JSON out. No maths.
  web/                    # Vite + vanilla TS, hand-rolled observable store, uPlot charts
    src/
      state.ts
      wasm/pkg/           # wasm-pack output — under src/, NOT public/ (playbook §6)
      worker/             # ONE data worker (see §2.1)
      panels/             # one file per left-column section
      plots/              # uPlot wrappers
  data/                   # real .mpt files — GITIGNORED (see §13.4)
  netlify.toml
```

### 2.1 Concurrency: simpler than hetdma

`hetdma-web` needed a worker *pool* because it ran a global optimisation. ICI does not.
Measured on a real file (`Gen 1 C2 BOL Cell 423 N23BM3_03_MB_C13.mpt`): 41,818 rows,
38 columns, 502 rest periods, ~51 points per rest. The full analysis is 502 two-parameter
OLS fits plus a handful of spline fits — well under a second in release-mode Rust. The
*expensive* parts are parsing 25 MB of text and drawing 42k points.

Therefore:

- **One dedicated Web Worker** owns the parsed dataset and runs parse + Stage A + Stage B
  (§7–§9). Parsing must not block the UI thread.
- **The parsed data stays in the worker.** Do not ship the full columnar dataset back to the
  main thread. The main thread receives: the column list, a decimated preview series for the
  raw plot, the raw-data table page currently being viewed, and the results tables.
- **A second, always-resident WASM instance on the main thread** handles the fast
  single-rest preview fit (§10.2) — the E vs √t diagnostic panel must update at interactive
  rates as `tmin`/`tmax` are dragged, exactly as the playbook's live-preview pattern
  describes. Ship it just the ~51 (t, E) points of the selected rest.
- **Message passing only. No `SharedArrayBuffer`, no COOP/COEP headers.** This is what keeps
  deployment to any static host trivial. Use transferable `ArrayBuffer`s for the arrays that
  do cross the boundary.

### 2.2 Internal unit convention

`core` works exclusively in these units. All conversion happens once, at import, in the
column-mapping step. Nothing downstream ever sees instrument units.

| Quantity | Internal unit |
|---|---|
| time `t`, `step.t` | s |
| voltage `E` | V |
| current `I` | A |
| charge `Q` | Ah |
| resistance `R` | Ω |
| `k` | Ω s^-1/2 |
| electrode area | cm² |

Display units are a presentation-layer concern only (§11.5).

---

## 3. Input data: what the files actually look like

**The general contract, which is what the parser must actually implement:** an optional
preamble of arbitrary length (possibly zero lines), followed by an optional single column-
header line, followed by numeric rows delimited by tab or comma. Nothing beyond that may be
assumed. The parser is **instrument-agnostic** — there are no per-vendor code paths, no
format sniffing by filename or magic string, and no vendor metadata is read or retained
(§4.2, §14 item 11).

The files in `data/` are one concrete instance of that contract: Bio-Logic EC-Lab ASCII
exports (`.mpt`). They are useful as a stress case precisely because they sit at the awkward
end of the space. Verified properties — **treat these as one test case, not as the spec**:

- **Encoding:** Latin-1 / cp1252, *not* UTF-8 (the preamble contains byte `0xB2`).
  Decoding as UTF-8 will fail or mojibake.
- **Line endings:** CRLF.
- **Preamble:** 112 lines of free-form text, then the column-header line at 113, then data
  from line 114. The preamble includes lines that are themselves tab-delimited tables with a
  *different* column count from the data — which is exactly what defeats a naive
  "first line with N tabs wins" heuristic.
- **Delimiter:** tab. Every row ends with a trailing tab, producing a spurious empty 39th
  field — the real column count is 38.
- **Numbers:** `.` decimal point in these files, scientific notation like
  `2.901364069601998E+005`. The same instrument in a European locale emits `,` as the decimal
  separator — handle it generically (§4.3).
- **Useful columns:** `time/s`, `Ewe/V`, `I/mA`, `(Q-Qo)/mA.h`,
  `Q charge/discharge/mA.h`, `cycle number`, `half cycle`, `Ns`, `step time/s`, `mode`,
  `ox/red`, `x`, `Capacity/mA.h`.
- **Rests:** current is *exactly* `0` during rest (25,602 of 41,818 samples), and the
  smallest non-zero |I| is 0.71 mA. So `state_threshold = 0` works, but keep it
  user-adjustable — other instruments won't be this clean.
- **Rest geometry:** 502 rests, each 9.952 s long, 51 points at 0.2 s sampling.
- **Extent:** 43.4 h, cycle number 0–2, half cycle 0–4.

**Other formats that must work without code changes**, and which belong in the parser test
suite as synthetic fixtures: a plain `.csv` with a header row and no preamble; a tab-delimited
`.txt` with no header row at all (columns then get synthetic names and the user maps them by
position); a file with a two-line preamble; a file with a comma delimiter and a comma decimal
separator (i.e. semicolon-delimited); LF-only line endings; a UTF-8 file with a BOM.

**Note the default R regression window is `c(2, 14)` s but these rests are only ~10 s.**
The R code silently uses whatever points fall inside the window, so it effectively fits
2–9.95 s. This is one reason the "estimate optimal window" feature (§10) matters.

---

## 4. Import

### 4.1 UI

Left column, top section. Drag-and-drop zone with click-to-browse fallback. One file at a
time in v1, but see §4.5.

On drop: show filename, size, and a parse progress indicator (25 MB files take a moment).
On success: show a compact **parse card** stating what the sniffer decided — encoding,
delimiter, decimal separator, number of preamble lines skipped, whether a header row was
found or synthesised, rows, columns — with a manual override control for each of those five
decisions. On failure: show the same card populated with what the sniffer concluded at each
step and where it gave up, so the user can diagnose an odd file rather than seeing
"parse error".

The parse card reports **structural** facts only. No preamble content is parsed, displayed,
stored or exported (§14 item 11).

### 4.2 Format detection algorithm

Fully generic. **No vendor-specific branches** — the algorithm must reach the right answer on
the EC-Lab files with no knowledge that they are EC-Lab files, and equally on a bare two-column
CSV. Implement in `core/src/parse.rs` so it is unit-testable natively.

1. **Decode.** Strip a UTF-8 BOM if present and decode UTF-8; on invalid sequences fall back
   to cp1252. Record which was used. Normalise CRLF and lone CR → LF.
2. **Choose a delimiter and locate the data block, jointly.** These two decisions are coupled
   and must be made together — deciding either one first is what makes naive sniffers fail on
   files whose preamble is itself delimited. For each candidate delimiter in
   `[\t, ,, ;, |]` and each candidate decimal separator in `[., ,]` (skipping the
   comma/comma combination):
   - Split every line (up to a cap of ~5,000 lines, or the whole file if smaller) and record
     its field count.
   - Find the **longest terminal run** of consecutive lines that all have the same field
     count `F ≥ 2` and in which every field is a number under the candidate decimal
     separator, treating empty, `NA`, `NaN`, `Inf`, `-Inf`, `null` and `-` as valid missing
     tokens. "Terminal" means the run must extend to the end of the sampled region — data
     blocks run to the end of file; preambles do not.
   - Score the combination as `run_length × F`, rejecting any with `run_length < 5` or
     `F < 2`.
   Take the highest-scoring combination. If nothing scores, fail with a clear message naming
   the delimiters tried.
3. **Locate the header row.** The candidate is the line immediately preceding the data run.
   Accept it as the header if it splits to the **same field count** `F` under the chosen
   delimiter and is **not** entirely numeric. Otherwise there is no header row: synthesise
   `col1…colF` and raise a visible (non-blocking) notice, since the user then has to map
   columns by position. Everything above the header — however many lines, zero included — is
   the preamble and is **discarded**; only the skipped-line count is retained.
4. **Trailing-empty-column fix.** If every data row has a trailing empty field (a trailing
   delimiter), and the header either has the same trailing empty field or one fewer field,
   drop that phantom column and log it.
5. **Ragged rows.** If some rows in the data block have a different field count from `F`
   (e.g. a truncated final line from an interrupted export), drop them, and report the count
   and first few line numbers as a warning rather than failing.
6. **Header cleanup.** Trim whitespace. Replace an empty header cell with `colN`. De-duplicate
   by suffixing ` (2)`, ` (3)`.
7. **Parse body** into a columnar store (`Vec<f64>` per column, NaN for unparseable). Keep a
   count and the first ten row indices of coercion failures per column and surface them as a
   warning, not an error.

Store columns in a struct that keeps name, values, and an `is_numeric` flag. Columns that are
predominantly non-numeric are retained as string columns, usable only as grouping columns.

**Manual override.** Every decision above is overridable from the parse card: delimiter,
decimal separator, encoding, "skip N lines", and "first row is a header" (yes/no). Changing
any of them re-parses. This is the escape hatch for the file format nobody anticipated, and it
is cheaper than trying to make the sniffer perfect.

### 4.3 Decimal comma

Handled inside the joint search in §4.2 step 2 rather than as a post-hoc fix-up, because the
decimal separator changes which lines count as numeric and therefore where the data block
appears to start. Surface the decision in the parse card with a manual override
(`.` / `,` / auto).

### 4.4 Guardrails

Reject files > 250 MB with a clear message. Warn (don't reject) above 100 MB. Parse
incrementally so a large file doesn't spike memory beyond ~3× file size.

### 4.5 Multi-file readiness (structure now, UI later)

Do not build multi-file UI in v1, but structure state so it costs little later:

- `AppState.datasets: Dataset[]`, v1 has length ≤ 1; all panels read `activeDataset`.
- `core` already supports `grouping_columns`. Reserve a synthetic `file_id` column that is
  injected into every dataset at parse time and becomes the first grouping column when more
  than one dataset is loaded.
- Analysis config is a top-level object, not per-dataset; column *mapping* is per-dataset
  (different instruments, different headers).
- Worker protocol messages carry a `datasetId`.

---

## 5. Column mapping and units panel

Appears immediately after a successful import. A table with one row per required field:

| Field | Data column | Units |
|---|---|---|
| Time `t` | *dropdown* | s / min / h |
| Cycle `cyc.n` | *dropdown* | (none) |
| Current `I` | *dropdown* | A / mA / µA |
| Voltage `E` | *dropdown* | V / mV |
| Charge `Q` | *dropdown* | Ah / mAh / µAh / C |

Plus an optional-fields sub-table: `step` (informational), and **grouping columns**
(multi-select, from non-mapped columns; empty in the single-file case).

### 5.1 Auto-mapping

On import, pre-populate mapping and units from the header names, case-insensitively:

| Field | Header patterns (in priority order) | Unit inferred from |
|---|---|---|
| time | `^time/s$`, `^t$`, `^time`, `^elapsed` | `/s` suffix → s |
| voltage | `^Ewe/V$`, `^E$`, `^ecell`, `voltage` | `/V`, `/mV` |
| current | `^I/mA$`, `^I$`, `^current` | `/mA`, `/A` |
| charge | `^\(Q-Qo\)/mA\.h$`, `^Q$`, `^capacity`, `charge` | `/mA.h`, `/A.h` |
| cycle | `^cycle number$`, `^cyc\.?n$`, `^cycle`, then `^half cycle` **last** | — |

Show pre-populated rows with a subtle "auto-detected — verify" marker that disappears the
moment the user touches that row (playbook §4).

**Cycle column.** A full cycle number (with charge and discharge distinguished by the `state`
the app derives itself) is the intended mapping and must be preferred by the auto-mapper. A
half-cycle column is also a legitimate choice and must not be blocked — it simply makes the
Q-anchoring and smoothing groups finer, which is harmless. Where both are present, pick the
full cycle and note the alternative in the dropdown ordering.

**Pre-normalised charge columns.** Some exports carry both a cumulative charge column and one
that has already been reset per half-cycle. Prefer the cumulative one, since the app performs
its own Q anchoring (§6). Detect the pre-normalised case generically — if the candidate charge
column returns to zero at the start of every `(cyc.n, state)` group — and show an info note
that anchoring will be a no-op, rather than hard-coding vendor column names.

Persist a mapping preset in `localStorage`, keyed by a hash of the header row, so re-loading
a file from the same instrument restores the mapping instantly.

### 5.2 Raw data inspector

Right-hand or bottom-docked collapsible panel (match hetdma's raw-data viewer placement).
Virtualised table, paged from the worker, showing all columns with the mapped ones
highlighted and labelled with their post-conversion internal units. Include a jump-to-row
box and a "show rows around rest #N" action. Also show per-column summary stats (n finite,
min, median, max) so the user can spot a wrong unit selection at a glance.

### 5.3 Validation, relaxed from R

`validate_ici_columns()` in R hard-errors on any non-finite value in a mapped column and on
non-monotonic time. In an interactive app that is too brittle. Instead:

- **Non-finite values in a mapped column** → warning naming the column and count, with a
  "drop these rows" action (default: drop, on, with the count shown).
- **Non-monotonic time** → warning showing the first offending row index, with actions
  "sort by time" or "drop the decreasing rows". Do not proceed to Stage A until resolved.
- **Duplicate column assignment** → hard block (as in R).
- **Non-numeric mapped column** → hard block.

Log every automatic row removal into the run log (§12.3).

---

## 6. Q normalisation (anchoring)

R sets `Q ← Q − Q[first]` within each `(grouping, cyc.n, state)` group, i.e. Q = 0 at the
*start* of each half-cycle.

**Spec:** per-state start/end anchor.

UI, in the import section under the mapping table:

- `Q = 0 at:` for **charge**: `start of half-cycle` (default) | `end of half-cycle`
- `Q = 0 at:` for **discharge**: `start of half-cycle` (default) | `end of half-cycle`

Implementation: within each `(grouping, cyc.n, state)` group, subtract either the first or
the last value of the mapped charge column, per the state's setting. `start/start`
reproduces R exactly and must be the default.

Show a one-line plain-English echo of the current choice, e.g.
*"Charge: Q = 0 at the discharge limit. Discharge: Q = 0 at the charge limit."* — the mapping
between "start of half-cycle" and "which electrochemical limit that is" is exactly the sort
of orientation confusion the playbook §6 warns about, so state it explicitly.

Anchoring applies to the segmented data used for everything downstream, and to `Q` in the
per-interruption summary.

---

## 7. Stage A — segmentation and resistance fitting

The pipeline is split into two independently re-runnable stages so that changing a smoothing
parameter doesn't re-run 502 regressions, and so the two have separate, clearly-labelled
parameter groups in the UI.

**Stage A inputs:** state threshold, regression window, interpolation/averaging windows,
edge points, drop flags, legacy-compatibility flag, extra summaries.
**Stage A outputs:** `segmented`, `summary`, `regression`, and the merged per-interruption
table with `R`, `k`, `R_err`, `k_err`.

Implement faithfully to `ici_analysis.R` lines 699–1032. The steps, with the details that
are easy to get wrong called out:

### 7.1 State classification

```
state = "R"          if |I| <= state_threshold
        "charge"     if I > 0
        "discharge"  if I < 0
```

`state_threshold` in A, default `0`, must be finite and ≥ 0.

If a group ends up with no rests or no active samples, run `split_current_levels()`
(R lines 264–305: an exact 1-D two-cluster split minimising total within-cluster SSE over
sorted |I|) and, if the two levels are clearly separated, **suggest** the midpoint threshold.
In R this is a hard error; in the app it should be a blocking banner with a
"use suggested threshold: X A" button. Port `split_current_levels()` verbatim, including the
`gap > 3·max(sd)` and `high > 1.5·low` separation criteria.

### 7.2 Rest indexing — read this carefully

```
starts = [false] ++ (state[i-1] == "R" && state[i] != "R")
rest   = cumsum(starts) + 1
```

computed **within each group**. The consequence, which the merge in §7.7 depends on: an
active segment and the rest period that *follows* it share the same `rest` id. Example:

```
sample:  R R A A R R A A
rest:    1 1 2 2 2 2 3 3
```

Get this wrong and every `R` and `k` will be paired with the wrong relaxation.

### 7.3 Step time

`step.t = t − t[first]` within `interaction(group, rest, state)`.

### 7.4 Dropping unrested reversals

If `drop_unrested_reversals` (default `true`): find every index pair where two adjacent
samples are both active with *different* states — a direct current reversal with no rest in
between. Walk backwards from that boundary to the start of the earlier active segment and
mark the whole segment for removal. Port `find_unrested_reversal_rows()` (R lines 354–376)
exactly, including that it operates per group. Log the count removed.

### 7.5 Dropping the incomplete final step

Per group: if the last sample's state is not `"R"`, remove all rows whose `rest` equals
`max(rest)` for that group. Log the count.

After 7.4 and 7.5, re-check that every group still has both rests and active samples; if not,
surface a blocking error naming the group.

### 7.6 Per-interruption summary (active segments)

Group active samples by `interaction(group, cyc.n, state, rest)`. For each:

| Output | Definition |
|---|---|
| `t` | last `t` in the segment |
| `step.t` | last `step.t` in the segment |
| `E` | `interpolate_endpoint(step.t, E, voltage_interpolation_window)` |
| `I` | mean of `select_final_window(step.t, I, current_average_window)` |
| `Q` | last (anchored) charge in the segment |
| extras | per §7.9 |

`interpolate_endpoint` (R 582–601): take finite points with `step.t ≥ max(step.t) − window`;
if fewer than 2, or fewer than 2 distinct `step.t`, return the voltage at the last point;
otherwise linear `approx` (with `rule = 2` clamping) evaluated at `max(step.t)`.
With `window = NULL`, return the last point.

`select_final_window` (R 548–556): finite points with `step.t ≥ max(step.t) − window`;
falls back to all finite points if that selects none; with `window = NULL` returns the single
point at `max(step.t)`.

`legacy_compatibility` (default `false`, keep as an advanced toggle): when true, `I` is the
mean of **all** finite current values in the active segment rather than the final-window
mean.

Drop R's `Efirst` output — it is an exact duplicate of `E`.

### 7.7 Per-rest regression

Group rest samples by `interaction(group, rest)`. For each:

- `I0 = mean(select_initial_window(step.t, I, current_average_window))` — the *initial*
  window (R 565–573), mirroring `select_final_window` at the other end.
- Fit points: indices where `step.t` is finite, `regression_window[0] ≤ step.t ≤
  regression_window[1]`, and `E` is finite.
- Fewer than 3 fit points → in R this is a fatal error. In the app: mark the rest as failed
  with a reason, exclude it, and add it to the run log; only error out if *every* rest fails.
- Sort by `x = sqrt(step.t)`, then OLS `E ~ x`.
- Outputs: `E0` (intercept), `E0_err`, `s` (slope), `s_err`, `I0`, `n_pts`, `r2`, `adj_r2`,
  `rmse` (the regression `sigma`, i.e. residual standard error with n−2 df),
  `edge_mae_ratio`, `edge_max_z`.

Edge diagnostics (R 960–976): `edge_count = min(edge_points, floor(n/2))`, default
`edge_points = 3`. Edge indices are the first and last `edge_count` points **in x-sorted
order**; centre indices are the rest. `edge_mae_ratio = mean|resid_edge| / mean|resid_centre|`,
`edge_max_z = max|resid_edge| / sigma` (NA if sigma is not finite and positive). These detect
curvature at the ends of the window — the signature of a badly chosen `tmin`/`tmax`.

Standard errors must come from the usual OLS formulae; compute in a numerically stable way
(centred sums, or a QR/Cholesky on the 2×2 normal equations) rather than the naive
`Σx²  − (Σx)²/n` form, since `x = √t` values are all O(1–3) but `E` differences are ~mV.

### 7.8 Derived quantities

Merge summary and regression on `(grouping…, rest)`, sorted.

```
ΔI     = I − I0
R      = (E − E0) / ΔI
R_err  = E0_err / |ΔI|
k      = −s / ΔI
k_err  = s_err / |ΔI|
```

Non-physical handling (`nonphysical_to_na`, default `true`; `nonphysical_columns = ["R","k"]`):
any value that is non-finite **or negative** is set to NA along with its `_err` partner.
`R` and `k` are resistances and are **always positive** — a negative value is never a real
measurement (§13.3). Count them per column **and per state** and surface as a warning (R's
`warn_nonphysical`); negatives clustered in one state specifically indicate a sign or
orientation bug rather than noise, and the warning should say so. Expose the
`nonphysical_to_na` toggle in the UI (advanced) so the raw values can be inspected when
diagnosing exactly that.

**Playbook §5 applies here directly:** never let `NaN`/`Inf` cross the WASM boundary as a
bare float. Serialise missing values as JSON `null` deliberately and have the TS layer handle
`null` everywhere it formats a number, or use the finite-sentinel approach. Divisions by
`ΔI` are the obvious hazard; there will be others.

### 7.9 Extra summary columns

R's `extra_summaries` lets arbitrary input columns be summarised over the final window of
each active step. Expose as an "Extra summary columns" list in the UI: pick a source column,
an output name (defaulted from the source), and an aggregator from
`mean` (default) / `median` / `first` / `last` / `min` / `max` / `sd`. Applied over
`select_final_window(step.t, values, current_average_window)`. Useful for carrying
temperature, `Ns`, `x`, `half cycle` etc. through into the results table.

---

## 8. Stage B — smoothing and derivatives

**Stage B inputs:** smoothing group key, three smoother configs, derivative window/degree,
QC exclusions.
**Stage B outputs:** `E0_smooth`, `R_smooth`, `k_smooth`, `dVdQ`, `dQdV` added to the
analysis table.

Re-running Stage B must not re-run Stage A. The UI shows two buttons: **Fit resistances**
(Stage A) and **Smooth & derive** (Stage B), with Stage B auto-running after Stage A and
also on any Stage-B parameter change (debounced ~300 ms).

### 8.1 Grouping

R smooths within `interaction(grouping_columns, cyc.n, state)`. Make the smoothing key
**visible and editable**: a chip list showing the active key, with `cyc.n` and `state`
toggleable and any grouping column toggleable. Default = R's behaviour, all on.

Directly above it, a read-only **"Grouping summary"** card that states, in plain terms, which
key is used at each stage — this was an explicit requirement:

```
Segmentation & rest indexing   : <grouping columns>  (whole file if none)
Q anchoring                    : <grouping>, cyc.n, state
Interruption summary           : <grouping>, cyc.n, state, rest
Rest regression                : <grouping>, rest
Smoothing & derivatives        : <editable smoothing key>
```

### 8.2 The smoother

This is the largest and highest-risk piece of the port. **Decision taken: implement the
faithful monotonic P-spline.**

Port `smooth_bspline()` (R 74–209) as used by `smooth_bspline_vec()` (R 15–58):

Preprocessing, per group:
1. Keep points where both `x` and `y` are finite.
2. **Average duplicate x values** (`aggregate(.y ~ .x, mean)`), then sort by `x`.
3. Require ≥ 8 distinct x values; otherwise return all-NA for the group and warn (R warns at
   `< 8` valid observations and errors at `< 8` distinct x — in the app both are warnings
   that yield NA).
4. `k ← min(k_requested, n_distinct − 2)`.

Fit:
- **Unconstrained** (`monotonic = false`, basis `"ps"`): cubic B-spline basis of dimension
  `k` on a uniform knot sequence over the data range, with an order-`m` difference penalty
  (`m = 1` by default in the ICI configs). Smoothing parameter `λ` chosen by REML.
- **Monotonic** (`monotonic = true`, basis `"mpi"` increasing / `"mpd"` decreasing): scam's
  SCOP-spline reparameterisation — B-spline coefficients constrained monotone by writing
  `β_j = β_1 + Σ_{i=2..j} exp(γ_i)`, with the difference penalty applied to `γ`, fitted by
  penalised iteratively-reweighted least squares with an outer optimisation over `λ`.
- **Direction, when `"automatic"`** (R 130–140): compute all pairwise slopes
  `(y_i − y_j)/(x_i − x_j)`, take the finite ones, and choose `"increasing"` if their
  **median** ≥ 0 else `"decreasing"`. This is O(n²) in R; for a few hundred interruptions
  that is fine, but use a subsample above ~2000 points and note it in the log. Reproduce the
  median-of-pairwise-slopes rule exactly, not a cheaper proxy — it decides the basis and
  therefore the whole fit.

Prediction: `smooth_bspline_vec` predicts at the **original valid x positions**, not on the
internal grid. The grid, `derivative_1` and `derivative_2` machinery inside `smooth_bspline`
is unused by the ICI pipeline — **do not port it.** Only `fit` + `predict` are needed.

Suggested crates: `nalgebra` or `faer` for the linear algebra; `argmin` or a hand-rolled
Brent search for the 1-D `λ` optimisation. Avoid pulling in anything that won't compile to
`wasm32-unknown-unknown`.

Defaults, matching R:

| Smoother | monotonic | direction | k | m |
|---|---|---|---|---|
| `E0` | true | automatic | 50 | 1 |
| `k` | false | automatic | 50 | 1 |
| `R` | false (inherits `k` config) | automatic | 50 | 1 |

In the UI, `R` smoothing gets an "inherit from k smoothing" checkbox, checked by default,
mirroring R's `r_smoothing = NULL` behaviour.

Each smoother panel exposes: monotonic on/off, direction (automatic/increasing/decreasing),
`k` (basis dimension), `m` (penalty order). Report the *effective* `k` used after the
`min(k, n−2)` clamp, and the selected `λ` and effective degrees of freedom, in the run log —
these are the first things to check when a smooth looks wrong.

### 8.3 Derivatives

Port `local_poly_derivative()` (R 219–257) exactly:

- `window` odd, default 5; `degree` default 3; require `window ≥ degree + 1`.
- Work on finite points sorted by `x`; return NA everywhere if fewer than `window` valid.
- For each point `i`: window start `= max(1, min(i − ⌊window/2⌋, n − window + 1))` — note the
  clamping at both ends means end points reuse the terminal window rather than shrinking it.
- Fit `y ~ poly(z, degree, raw = TRUE)` where `z = x − x_i`; the derivative is the linear
  coefficient. On fit failure, NA.
- Un-sort back to input order.

Applied as:

```
dVdQ = local_poly_derivative(Q, E0_smooth, derivative_window, derivative_degree)
dQdV = 1 / dVdQ
```

`dVdQ`/`dQdV` are retained. `E_eq`, `dEeqdQ`, `dQdE_eq` and the legacy aliases
(`dEdQ`, `dQdE`, `ocv`, `dEdocv`, `docvdE`) are **dropped** — see §14.

---

## 9. Quality control

Computed in Stage A, applied in Stage B. Panel in the left column between the two stages.

**Per-rest flags**, each with a user-editable threshold and an on/off toggle:

| Flag | Default rule |
|---|---|
| Poor fit | `adj_r2 < 0.98` |
| Too few points | `n_pts < 5` |
| Edge curvature | `edge_max_z > 3` |
| Edge/centre imbalance | `edge_mae_ratio > 2` |
| Non-physical | `R` or `k` was NA'd in §7.8 |
| Degenerate ΔI | `\|ΔI\| < 1e-9` A |

Behaviour:
- Flagged rests are **highlighted** in the results table and drawn in a muted/warning colour
  on the R and k plots.
- An **"Exclude flagged rests from smoothing and export"** checkbox (default **on** for
  non-physical and degenerate ΔI, **off** for the rest — so the default is close to R while
  making the escape hatch obvious).
- A manual per-rest exclude/include toggle in the results table that survives Stage-B re-runs
  (keyed by group + rest id).
- A summary line: *"502 rests · 7 flagged · 3 excluded"*, click-through to filter the table.

**Click-through:** clicking any point on the R vs Q or k vs Q plot (or a row in the results
table) selects that rest and loads it into the E vs √t diagnostic panel (§10.2). This is the
main workflow for deciding whether a flagged point is real or a bad fit.

---

## 10. The regression window

### 10.1 Manual entry

Two numeric inputs, `t_min` and `t_max`, in seconds, with validation
`0 ≤ t_min < t_max`. Default `2` and `14` (R's default). Show, live, how many points fall in
the window for the currently-selected rest and the median across all rests — a window that
yields 3 points is a silent disaster otherwise.

### 10.2 The E vs √t diagnostic panel

Plot 2 in the grid (§11.2). Shows the selected rest's `(√step.t, E)` points, the fitted line
drawn across the window, the excluded points outside the window shown greyed, and shaded
window bounds that can be **dragged directly on the plot** (dragging updates the inputs, and
vice versa). Annotated with:

```
R  = 12.34 ± 0.11 Ω
k  = 0.4567 ± 0.0031 Ω s^-1/2
adj R² = 0.9987     n = 41     RMSE = 0.21 mV
```

This runs on the main-thread WASM instance for immediate feedback while dragging (§2.1).

A rest navigator sits above it: `◀ Rest 137 of 502 ▶`, a jump-to box, a
"next flagged rest" button, and keyboard `←`/`→` bindings.

### 10.3 "Estimate optimal window"

A button below the two inputs. Runs in the worker with a progress bar and a cancel button.

**Sampling.** Choose `N` rests (default 20, user-adjustable 5–100) well distributed across
the charge range: take the `Q` value of each rest's preceding interruption, split
`[Q_min, Q_max]` into `N` equal bins, and take the rest nearest each bin centre; if a bin is
empty, take the next-nearest unused rest. Stratify **within each state** if both charge and
discharge rests exist, so the sample isn't dominated by one direction. Report which rests
were sampled.

**Candidate grid.** Let `T = ` the maximum `step.t` observed across the sampled rests
(use the 5th percentile of per-rest maximum `step.t` if rest lengths vary, so candidates
remain valid for nearly all rests — report if lengths are heterogeneous). Then:

```
t_min ∈ {1, 2, …, floor(T) − L_min}
t_max ∈ {t_min + L_min, …, floor(T)}
```

with `L_min` = minimum window length, default **5 s**, exposed as an advanced setting, and
`t_min`'s lower bound exposed too (default 1 s, allow 0). For the example data
(`T ≈ 9.95 s`, so `floor(T) = 9`) this is a grid of ~10 candidates; for 30 s rests, ~325.
Either way it is trivially cheap — 325 × 20 = 6,500 two-parameter OLS fits.

**Scoring.** For each candidate, fit every sampled rest; discard rests with fewer than
`max(4, 3)` points in that window; the score is the **mean adjusted R²** across the rests
that fitted successfully. Reject a candidate outright if fewer than 80 % of the sampled rests
fitted.

**Result.** The winner is the highest mean adjusted R². Show a results panel with the top 10
candidates in a small table (`t_min`, `t_max`, mean adj R², median adj R², n valid rests,
median n points) and an "Apply" button on each row, plus a small heatmap of mean adj R² over
the `(t_min, t_max)` grid — the shape of that surface is more informative than the single
winner.

**Caveat to display in the panel, one line:** adjusted R² is being compared across fits to
*different subsets of points*, so it ranks windows heuristically rather than as a formal
model comparison; the median adj R² and the edge diagnostics are worth eyeballing before
accepting. Include median `edge_max_z` as a secondary column for exactly this reason.

Ties (within 1e-6 mean adj R²) break towards the longer window, then the smaller `t_min`.

---

## 11. Visualisation

Main area, **two-column grid of plots**, uPlot, synced cursor across panels sharing an x
column, matching hetdma-web's look and feel. Desktop only, no responsive breakpoints.

### 11.1 Plot 1 — raw time series (available immediately after import)

`E` vs `t`, full record. Optional second y-axis for `I`. Rest periods lightly shaded. The
current regression window's selected rest is marked. **Decimate with LTTB** to ~4,000 points
for display, re-decimating on zoom so detail returns as you zoom in — 42k points per series
is already sluggish and multi-file will be worse.

### 11.2 Plot 2 — E vs √t for the selected rest

As specified in §10.2.

### 11.3 Plots 3 and 4 — R and k (available after Stage A/B)

`R` vs x and `k` vs x, where x defaults to `Q` and is selectable from any numeric column in
the analysis table (`E`, `E0`, `t`, `cyc.n`, `x`, extras…). Each shows:

- data points, coloured/split by `state` (and by group when grouping is active),
- **error bars** from `R_err` / `k_err`, with a toggle (default on) and a
  "hide bars smaller than N px" option so dense plots stay readable,
- the **smoothing line** from `R_smooth` / `k_smooth`, one line per smoothing group,
- flagged points muted, excluded points hollow,
- click-to-select-rest (§9).

### 11.4 Additional plots

An **"+ Add plot"** button below the grid. Each added panel gets x and y column dropdowns
(any numeric column in the analysis table, plus `E0_smooth`, `R_smooth`, `k_smooth`, `dVdQ`,
`dQdV`), a points/line/both selector, an optional error-bar column, and a remove button.
Panels are reorderable by drag. Panel configuration is part of the exported config (§12.2).

### 11.5 Axes and area normalisation

Every panel has: x-min / x-max / y-min / y-max numeric inputs with an **Auto** button per
axis, and optional log-scale toggles. Follow the playbook §6 uPlot rule precisely — always
`setData(data, true)` then immediately re-apply stored custom ranges with
`setScale(...)`; never pass `resetScales = false`.

**Electrode area.** A numeric field in the visualisation section of the left column, in cm²,
with a "normalise to area" master toggle (default off). Entered manually — the app does not
read it from instrument metadata (§14 item 11). Default `1.0`, so the toggle is a no-op until
a real area is entered; show a warning marker on the toggle while the area is still `1.0`.
The value is remembered in the session-restore config (§16 item 6).

When normalisation is on, affected quantities are displayed and labelled as:

| Quantity | Normalised | Label |
|---|---|---|
| `R`, `R_smooth`, `R_err` | `× A` | Ω cm² |
| `k`, `k_smooth`, `k_err` | `× A` | Ω cm² s^-1/2 |
| `Q` | `÷ A` | mAh cm⁻² |
| `dVdQ` | `× A` | V cm² mAh⁻¹ |
| `dQdV` | `÷ A` | mAh V⁻¹ cm⁻² |

**Normalisation is presentation-only.** It must never touch the fitting, the smoothing, or
the internal-unit data. Apply it at the plotting/table-formatting layer. Exported analysed
data is written in internal units (§12.1) with the area recorded in the metadata; offer an
"apply area normalisation to export" checkbox, off by default.

### 11.6 Summary statistics

Below the plot grid, a stats block reporting, per smoothing group:
`n` interruptions, `n` flagged/excluded, median and IQR of `R` and `k`, median `adj R²`,
median `n_pts`, `R` and `k` at the start/mid/end of the Q range, Q range, and the fitted
`λ`/EDF for each smoother. Plus the full analysis table (virtualised, sortable, filterable,
with the QC toggles from §9).

---

## 12. Export

All exports are client-side blob downloads with a filename field the user can edit
(pre-filled from the source filename plus a suffix).

### 12.1 Analysed data — tab-delimited plain text

`.txt` (or `.tsv`), tab-delimited, UTF-8, LF line endings, one header row. Columns: grouping
columns, `cyc.n`, `state`, `rest`, `t`, `step.t`, `E`, `I`, `Q`, `E0`, `E0_err`, `s`, `s_err`,
`I0`, `n_pts`, `r2`, `adj_r2`, `rmse`, `edge_mae_ratio`, `edge_max_z`, `R`, `R_err`, `k`,
`k_err`, `E0_smooth`, `R_smooth`, `k_smooth`, `dVdQ`, `dQdV`, extras, `flags`, `excluded`.

Missing values written as empty fields (with a `NA` / empty / `NaN` selector, default empty).
Numeric precision: full `f64` round-trip precision by default (`{:.17e}`-equivalent shortest
round-trip), with a "6 significant figures" option for human reading. Internal units, with a
commented unit line optionally prepended.

Secondary exports behind a dropdown: the per-rest regression table, and the segmented data
(this one is large — warn).

### 12.2 Configuration — JSON

Everything needed to reproduce a run from the same input file: schema version, column
mapping and units, Q anchoring, all Stage A parameters, all Stage B parameters, QC thresholds
and manual exclusions, smoothing group key, electrode area, and the plot panel definitions.
**No data.** Must be re-loadable via a "Load config" button that validates the schema version
and reports any settings that couldn't be applied (e.g. a mapped column absent from the
current file).

### 12.3 Run report — JSON

For a completed fit: the full config (as §12.2), plus

- **provenance:** filename, byte size, SHA-256 of the file, detected encoding, delimiter,
  decimal separator, number of preamble lines skipped, whether the header was found or
  synthesised, row and column counts, the column names, app version and git commit.
  **No instrument metadata** — no preamble content is retained anywhere in the app (§14
  item 11),
- **log:** timestamped, ordered list of every automatic decision and warning — rows dropped
  for non-finite values, unrested reversals removed, incomplete final step removed, rests
  that failed to fit and why, non-physical R/k counts, spline `λ`/EDF/effective `k`/direction
  chosen per group, optimal-window search result and top candidates,
- **key statistics:** everything in §11.6, machine-readable,
- **timings** per stage.

The log shown in the UI (a collapsible panel at the bottom of the left column, with a badge
when new warnings appear) and the log in this export are the same object.

### 12.4 Plot images

Per-panel **PNG export** in v1, via a small download button in each panel's corner. Render at
a user-selectable scale factor (1× / 2× / 3×, default 2×) rather than the on-screen canvas
size, so the output is usable in a document or slide. Include the axis labels and units as
displayed, honouring the area-normalisation state. Filename defaults to
`<source>_<y>_vs_<x>.png`.

SVG export is deferred — uPlot is canvas-based, so SVG needs a separate vector render path
and is a materially larger job than PNG. Flag it as a follow-up rather than attempting both.

An "export all panels" action producing a zip is a nice-to-have, not required.

### 12.5 Session restore

Persist the current config object (§12.2) to `localStorage` on every change, debounced. On
next load, if a stored config exists, show an unobtrusive banner: *"Restore your last
session's settings? (data must be re-imported)"* with Restore and Dismiss. Data is never
persisted — only settings.

Restoring before a file is loaded should populate every parameter panel and leave the column
mapping pending until a file arrives, at which point the stored mapping is applied if the
header signature matches (§5.1) and reported as unapplied if it does not.

---

## 13. Correctness strategy

### 13.1 Golden-file gate — blocking, before any UI work

Playbook §3 applies in full and is non-negotiable. Concretely:

1. Write `tools/make_fixtures.R` in this repo. It loads `ici_analysis.R`, reads a **trimmed**
   real `.mpt` (say 3,000 rows covering ≥ 20 rests — small enough to commit if Matt approves,
   see §13.4) plus one or two synthetic cases, and for **at least four** config vectors
   spanning the parameter space writes JSON containing:
   - the cleaned, unit-converted input columns (so parsing can be validated separately from
     the maths),
   - `segmented` (state, rest, step.t, anchored Q),
   - `summary`, `regression`, and the full `analysis` table.
2. **Precision:** write floats with `format(x, digits = 17)` or `jsonlite::toJSON(digits = NA)`.
   The playbook flags this specifically — `jsonlite` truncates to 4 digits by default and
   serialises `NA` as the string `"NA"` unless told otherwise. Use `na = "null"`.
3. In `core/tests/golden.rs`, load the fixtures and assert to explicit tolerances:
   - segmentation (state, rest, step.t, Q): exact for integers, 1e-12 relative for floats,
   - regression outputs (`E0`, `s`, errors, `r2`, `adj_r2`, `rmse`, edge diagnostics): 1e-9
     relative,
   - `R`, `k`, `R_err`, `k_err`: 1e-9 relative,
   - `E0_smooth`, `R_smooth`, `k_smooth`: 1e-6 relative — the spline port will not match
     scam/mgcv bit-for-bit, and the λ optimiser in particular may land marginally differently.
     **If it doesn't hit 1e-6, stop and report before loosening it further.**
   - `dVdQ`, `dQdV`: 1e-5 relative (they inherit spline error and amplify it).
4. **Gate all further work on this passing.**

### 13.2 Micro-fixtures

Small, hand-checkable unit tests for each high-risk step, so an end-to-end failure is
diagnosable:

- `find_unrested_reversal_rows` — hand-built state sequences including a reversal at the very
  first and very last sample.
- Rest indexing — the `R R A A R R A A → 1 1 2 2 2 2 3 3` case from §7.2, plus a record that
  starts active and one that ends active.
- `select_final_window` / `select_initial_window` / `interpolate_endpoint` — including the
  `window = NULL` path, the fewer-than-2-points fallback, and the all-identical-`step.t` case.
- `split_current_levels` — a clearly bimodal vector and a unimodal one.
- `local_poly_derivative` — on a cubic with a known analytic derivative, checking the end-point
  clamping behaviour specifically.
- Spline — on `y = x²` and a known monotone function with a known ideal fit, plus the
  duplicate-x averaging and the `k ← min(k, n−2)` clamp.
- The `automatic` direction rule — a case where the median pairwise slope and the endpoint
  slope disagree in sign, since that's where a cheap proxy would diverge.
- Parser — a synthetic file per hazard: decimal comma, cp1252 bytes, trailing tab, no header,
  comma-delimited, CRLF, a preamble whose `Nb header lines` count is wrong.

### 13.3 Sign-convention and round-trip test (independent of R)

Golden-file tests cannot catch a sign error that the R implementation shares. Two checks
close that gap:

**The physical invariant, stated once and enforced everywhere:**

> `R` and `k` are resistances. Both are **always positive**. A negative value is unphysical
> and indicates a sign, orientation or segmentation error — never a real measurement.

This is already why §7.8 NAs out negative values, but it must also be an explicit assertion
in the test suite and a visible diagnostic in the app:

- `core` unit tests assert `R > 0` and `k > 0` for every rest in the reference fixture, for
  **both** charge and discharge states. Discharge is the case where a sign convention error
  hides, because `ΔI` flips sign and a compensating error in `(E − E0)` or in `−s` can cancel
  out on charge only.
- The run log records the count of negative `R` and negative `k` **separately by state**. A
  result where negatives cluster in one state is a convention bug, not noisy data, and the
  log should say so in those words when > 20 % of one state's values are negative.

**Synthetic round-trip test.** Generate a dataset from known ground truth: pick
`R_true(Q)` and `k_true(Q)` as smooth positive functions, a pulse current `I_pulse`, and
build `E(t)` for each rest as `E_end − ΔI·R_true − ΔI·k_true·√t` plus small Gaussian noise,
with charge and discharge pulses alternating and `Q` advancing realistically. Run the whole
pipeline and assert recovery of `R_true` and `k_true` to within the noise-implied tolerance,
**separately for charge and discharge**. Run it again with the sign of the current column
flipped and assert the recovered `R` and `k` are unchanged and still positive.

This test is cheap, catches the entire class of orientation bug that the playbook §6 warns
about, and is the one test that would survive the reference implementation being wrong.

### 13.4 Browser verification

Per playbook §8: after each milestone, drive the real app in a real browser with a real
25 MB `.mpt` file. Numeric checks beat screenshots — e.g. assert that `R` at a chosen rest,
read off the UI, matches the R implementation's value for the same rest to the displayed
precision. Build and serve the **production** bundle (`npm run build` + `vite preview`) before
calling deployment done; worker bundling and WASM loading differ between dev and prod.

### 13.5 Repository hygiene

- `.gitignore` from the outset: `target/`, `node_modules/`, `web/dist/`, `web/src/wasm/pkg/`,
  and **`data/`** (proprietary experimental data).
- Excluding `data/` would disable the golden tests for anyone else cloning the repo, so a
  **trimmed, anonymised fixture is committed** under `core/tests/fixtures/`. **Approved, with
  conditions:**
  - **Filename must be anonymised** — no cell ID, project code, chemistry code, channel or
    date. Use `ici_reference_a.txt`, `ici_reference_b.txt`, etc.
  - **The preamble must be removed entirely**, not merely edited — it carries the operator's
    directory paths, machine serial numbers, setting-file names and timestamps. Committing a
    preamble-free file is both the safe option and a *better* parser fixture, since the
    preamble hazards are covered by synthetic fixtures anyway (§13.2).
  - Trim to a few thousand rows covering ≥ 20 rests, and shift `t` to start at zero so the
    acquisition timestamp cannot be reconstructed.
  - Strip columns not needed by the tests.
  - Still make the golden test **skip with a clear message** when a fixture is absent, so the
    larger untrimmed fixtures can stay local.
- Commit only when asked; small, single-purpose commits.

---

## 14. Deviations from `ici_analysis.R` (the complete list)

Everything here is an intentional, approved departure. Anything *not* here must match R.

1. **`E_eq` and its derivatives are removed.** `E_eq`, `dEeqdQ`, `dQdE_eq` are not computed
   and not exported — the analysis is not currently reliable. `dVdQ` and `dQdV` are retained.
2. **Legacy aliases removed.** `dEdQ`, `dQdE`, `ocv`, `dEdocv`, `docvdE` and the
   `include_legacy_aliases` flag are dropped.
3. **`Efirst` removed** from the interruption summary (exact duplicate of `E`).
4. **Q anchoring is configurable** per state (start or end of half-cycle). R's behaviour
   (start for both) is the default.
5. **Validation is relaxed from errors to warnings with actions** for non-finite values in
   mapped columns, non-monotonic time, and rests with fewer than three fit points (§5.3, §7.7).
   R's hard errors become blocking banners with suggested fixes only when the whole dataset is
   unusable.
6. **QC flagging, filtering and manual exclusion added** (§9). Not present in R.
7. **Optimal regression window estimator added** (§10.3). Not present in R.
8. **Area normalisation added** as a display-only transform (§11.5).
9. **The unused derivative/grid machinery inside `smooth_bspline` is not ported** — only the
   fit-and-predict path that `smooth_bspline_vec` actually uses.
10. **The spline is a port, not a binding.** `scam`/`mgcv` are not available in WASM; the
    SCOP-spline and P-spline are reimplemented, so small numerical differences are expected
    and bounded by the tolerances in §13.1.
11. **No instrument metadata is read or retained.** R never touched it, and the app must not
    either: no preamble parsing beyond counting the lines to skip, nothing shown in the UI,
    nothing in the config or run report, no vendor-specific code paths anywhere in the
    parser. The electrode area is entered by hand.
12. **`R > 0` and `k > 0` are enforced as a stated physical invariant**, asserted in the test
    suite and reported by state in the run log (§13.3), not merely NA'd silently.

Retained from R even though they may look like candidates for removal: `legacy_compatibility`
(advanced toggle), `nonphysical_to_na` and `warn_nonphysical` (advanced toggles),
`drop_unrested_reversals` (advanced toggle, default on), `extra_summaries` (§7.9).

---

## 15. Build order

Each milestone verified working — tests green, and for UI milestones actually driven in a
browser with a real file — before the next starts.

1. **Toolchain and scaffolding.** rustup + `wasm32-unknown-unknown` + wasm-pack; Node 20 via
   nvm (check the system version, it's often too old). Empty Cargo workspace + empty Vite
   project. Confirm `cargo test`, `wasm-pack build` and `npm run dev` all work before writing
   real code. Mind the playbook's `--out-dir`-is-relative-to-the-crate-path gotcha.
2. **Parser, generic and instrument-agnostic** (§4.2), with the synthetic format fixtures
   from §13.2 written *first* — a bare CSV, a headerless TSV, a two-line preamble, decimal
   comma, cp1252, BOM, ragged final row, trailing delimiter — then the 112-line-preamble case
   as the hardest instance rather than the design target.
3. **`make_fixtures.R` + golden gate for parsing and segmentation.** Match R's cleaned input
   and `segmented` output exactly.
4. **Regression + derived quantities, golden-gated.** `summary`, `regression`, `R`, `k`, plus
   the synthetic sign-convention round-trip test (§13.3). Now the maths that matters is
   provably right, and independently so.
5. **Spline + derivatives, golden-gated.** The big one. Budget accordingly.
6. **WASM boundary + import UI + parse card + column mapping + raw data inspector.** First
   real browser milestone.
7. **Plots 1 and 2 + the live single-rest preview path.** uPlot, main-thread WASM instance,
   draggable window bounds.
8. **Stage A/B parameter panels + run buttons + plots 3 and 4 + results table.**
9. **Optimal window estimator** (§10.3) — cheap once the regression path exists.
10. **QC panel + click-through + flagging** (§9).
11. **Additional plots, axis controls, area normalisation** (§11.4–11.5).
12. **Export: TSV, config JSON, run report JSON, plot PNGs; config load; session restore**
    (§12).
13. **Polish, production-bundle verification, Netlify deploy** (playbook §7).

---

## 16. Decisions on record

All open questions are resolved. Nothing in this section requires further input; it exists so
the implementer knows which choices were deliberate and must not be quietly revisited.

| # | Question | Decision | Where specified |
|---|---|---|---|
| 1 | Smoothing fidelity | **Faithful monotonic P-spline** — SCOP-spline reparameterisation with REML λ selection, ported not approximated | §8.2 |
| 2 | Q anchoring | **Per-state start/end anchor**; R's start/start is the default | §6 |
| 3 | Per-rest QC | **Flag + filter + click-through** | §9 |
| 4 | Cycle column | **Full cycle number preferred** by the auto-mapper; half-cycle allowed and harmless (it only makes the groups finer) | §5.1 |
| 5 | Committed fixture | **Yes**, trimmed — but **filename anonymised** and **preamble stripped entirely** | §13.5 |
| 6 | Instrument metadata | **No.** Nothing from the preamble is read, shown, stored or exported; no vendor-specific parser paths | §4.2, §14 item 11 |
| 7 | Threshold auto-suggestion | **Yes** — offer the `split_current_levels` midpoint as a one-click fix | §7.1 |
| 8 | Plot image export | **Yes**, PNG per panel in v1; SVG deferred | §12.4 |
| 9 | Session restore | **Yes**, settings only, never data | §12.5 |
| 10 | Multi-file overlay | **State shaped for it now, UI deferred** | §4.5 |
| 11 | Sign-convention test | **Yes** — synthetic round-trip, plus `R > 0` and `k > 0` enforced as a stated invariant asserted per state | §13.3, §14 item 12 |

### 16.1 The two things most likely to go wrong

Flagged here rather than buried, because they are where this build will actually cost time:

**The spline (§8.2).** Everything else is careful, well-defined work. The SCOP-spline with
REML smoothing-parameter selection is the one module where a competent implementation can
still land somewhere subtly different from `scam`. It is gated at 1e-6 relative on
`E0_smooth`. If it will not meet that, **stop and report** — do not loosen the tolerance to
make the suite green. A tolerance quietly relaxed from 1e-6 to 1e-3 is indistinguishable from
a passing test until someone trusts a `dVdQ` peak position that isn't there.

**The parser (§4.2).** The requirement is instrument-agnostic robustness, not "reads our
`.mpt` files". The tell that this has been built correctly is that the EC-Lab files are just
another entry in the fixture list, that no source file contains the string "EC-Lab", and that
every sniffer decision has a manual override in the UI. A parser that special-cases the one
format on hand will fail silently on the next instrument, and the failure mode — a preamble
line mistaken for a header, columns shifted by one — produces plausible-looking numbers
rather than an error.
