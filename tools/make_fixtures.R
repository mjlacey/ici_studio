#!/usr/bin/env Rscript
#
# Golden-file fixture generator (ICI_WEB_SPEC.md §13.1). Loads ici_analysis.R,
# reads the committed reference fixtures (core/tests/fixtures/), runs the
# reference implementation for a handful of config vectors spanning the
# parameter space, and writes full-precision JSON that core/tests/golden.rs
# checks the Rust port against.
#
# Usage: Rscript tools/make_fixtures.R

if (!requireNamespace("jsonlite", quietly = TRUE)) {
  stop("Package 'jsonlite' is required to write golden fixtures.", call. = FALSE)
}
if (!requireNamespace("mgcv", quietly = TRUE) || !requireNamespace("scam", quietly = TRUE)) {
  stop("Packages 'mgcv' and 'scam' are required (ici_analysis always smooths).", call. = FALSE)
}

source("ici_analysis.R")

fixtures_dir <- "core/tests/fixtures"

write_json <- function(obj, path) {
  jsonlite::write_json(
    obj,
    path,
    digits = NA,
    na = "null",
    auto_unbox = TRUE,
    pretty = TRUE
  )
  cat(sprintf("Wrote %s\n", path))
}

# ---------------------------------------------------------------------------
# Fixture A: trimmed real EC-Lab export (core/tests/fixtures/ici_reference_a.txt)
# Instrument units -> internal units done here, once, by hand (the app's own
# column-mapping step does this in later milestones; make_fixtures.R stands
# in for it so the golden JSON's "cleaned_input" is what core::parse +
# core::segment are checked against).
# ---------------------------------------------------------------------------
raw_a <- read.delim(file.path(fixtures_dir, "ici_reference_a.txt"), check.names = FALSE)
cleaned_a <- data.frame(
  t = raw_a[["time/s"]],
  cyc.n = raw_a[["cycle number"]],
  I = raw_a[["I/mA"]] / 1000,
  E = raw_a[["Ewe/V"]],
  Q = raw_a[["(Q-Qo)/mA.h"]] / 1000
)

# ---------------------------------------------------------------------------
# Fixture B: hand-built synthetic case (core/tests/fixtures/ici_synthetic_b.csv)
# Already expressed at internal-unit scale; exercises a group that starts
# active, an unrested current reversal, an incomplete final active step, and
# two groups via the "cell" grouping column.
# ---------------------------------------------------------------------------
raw_b <- read.csv(file.path(fixtures_dir, "ici_synthetic_b.csv"))
cleaned_b <- data.frame(
  cell = raw_b[["cell"]],
  t = raw_b[["t"]],
  cyc.n = raw_b[["cyc.n"]],
  I = raw_b[["I"]],
  E = raw_b[["E"]],
  Q = raw_b[["Q"]]
)

default_smoothing <- list(monotonic = FALSE, direction = "automatic", k = 8L, m = 1L)

run_case <- function(name, data, columns, grouping_columns, state_threshold,
                      regression_window, voltage_interpolation_window,
                      current_average_window, edge_points, drop_unrested_reversals,
                      legacy_compatibility) {
  cat(sprintf("Running case '%s'...\n", name))
  result <- ici_analysis(
    data = data,
    columns = columns,
    grouping_columns = grouping_columns,
    state_threshold = state_threshold,
    regression_window = regression_window,
    voltage_interpolation_window = voltage_interpolation_window,
    current_average_window = current_average_window,
    edge_points = edge_points,
    e0_smoothing = list(monotonic = TRUE, direction = "automatic", k = 8L, m = 1L),
    k_smoothing = default_smoothing,
    r_smoothing = NULL,
    derivative_window = 5L,
    derivative_degree = 3L,
    legacy_compatibility = legacy_compatibility,
    drop_unrested_reversals = drop_unrested_reversals,
    nonphysical_to_na = TRUE,
    warn_nonphysical = TRUE,
    nonphysical_columns = c("R", "k"),
    include_legacy_aliases = FALSE
  )

  config <- list(
    name = name,
    columns = as.list(columns),
    grouping_columns = if (is.null(grouping_columns)) list() else as.list(grouping_columns),
    state_threshold = state_threshold,
    regression_window = regression_window,
    voltage_interpolation_window = voltage_interpolation_window,
    current_average_window = current_average_window,
    edge_points = edge_points,
    drop_unrested_reversals = drop_unrested_reversals,
    legacy_compatibility = legacy_compatibility
  )

  list(
    config = config,
    cleaned_input = data,
    segmented = result$segmented,
    summary = result$summary,
    regression = result$regression,
    analysis = result$analysis
  )
}

cases <- list(
  golden_case_1 = run_case(
    "reference_a_defaults",
    cleaned_a,
    columns = c(time = "t", cycle = "cyc.n", current = "I", voltage = "E", charge = "Q"),
    grouping_columns = NULL,
    state_threshold = 0,
    regression_window = c(2, 14),
    voltage_interpolation_window = 10,
    current_average_window = 10,
    edge_points = 3L,
    drop_unrested_reversals = TRUE,
    legacy_compatibility = FALSE
  ),
  golden_case_2 = run_case(
    "reference_a_narrow_window_legacy",
    cleaned_a,
    columns = c(time = "t", cycle = "cyc.n", current = "I", voltage = "E", charge = "Q"),
    grouping_columns = NULL,
    state_threshold = 0,
    regression_window = c(1, 5),
    voltage_interpolation_window = NULL,
    current_average_window = NULL,
    edge_points = 2L,
    drop_unrested_reversals = TRUE,
    legacy_compatibility = TRUE
  ),
  golden_case_3 = run_case(
    "synthetic_b_grouped_drop_reversals",
    cleaned_b,
    columns = c(time = "t", cycle = "cyc.n", current = "I", voltage = "E", charge = "Q"),
    grouping_columns = "cell",
    state_threshold = 0,
    regression_window = c(0, 3),
    voltage_interpolation_window = NULL,
    current_average_window = NULL,
    edge_points = 1L,
    drop_unrested_reversals = TRUE,
    legacy_compatibility = FALSE
  ),
  golden_case_4 = run_case(
    "synthetic_b_grouped_keep_reversals",
    cleaned_b,
    columns = c(time = "t", cycle = "cyc.n", current = "I", voltage = "E", charge = "Q"),
    grouping_columns = "cell",
    state_threshold = 0,
    regression_window = c(0, 3),
    voltage_interpolation_window = NULL,
    current_average_window = NULL,
    edge_points = 1L,
    drop_unrested_reversals = FALSE,
    legacy_compatibility = FALSE
  )
)

for (case_name in names(cases)) {
  write_json(cases[[case_name]], file.path(fixtures_dir, paste0(case_name, ".json")))
}

cat("Done.\n")
