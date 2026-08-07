#!/usr/bin/env Rscript
#
# One-time (reproducible) prep step for the committed golden-file fixture.
# Reads a real, proprietary EC-Lab .mpt export from data/ (gitignored) and
# writes a small, anonymised, preamble-free reference file under
# core/tests/fixtures/ that is safe to commit -- see ICI_WEB_SPEC.md §13.5.
#
# Anonymisation per §13.5:
#   - output filename carries no cell ID/project code/chemistry/channel/date
#   - the entire instrument preamble is dropped, not merely edited
#   - trimmed to a few thousand rows covering >= 20 rests
#   - time shifted to start at zero (acquisition timestamp unrecoverable)
#   - only the columns actually needed by the tests are kept
#
# Usage: Rscript tools/trim_reference_fixture.R

source_file <- "data/Gen 1 C2 BOL Cell 423 N23BM3_03_MB_C13.mpt"
out_file <- "core/tests/fixtures/ici_reference_a.txt"
# Chosen so the trim ends exactly on a rest boundary (row 3133 is where the
# 38th rest would start): an R regression needs its rest to be fully
# captured, unlike an active step, which R itself drops if incomplete
# (§7.5) -- there is no equivalent "drop the incomplete final rest" rule, so
# truncating mid-rest would leave a too-short/degenerate final regression.
n_rows <- 3132L

# Useful columns per ICI_WEB_SPEC.md §3.
keep_columns <- c(
  "time/s", "Ewe/V", "I/mA", "(Q-Qo)/mA.h", "Q charge/discharge/mA.h",
  "cycle number", "half cycle", "Ns", "step time/s", "mode", "ox/red", "x",
  "Capacity/mA.h"
)

lines <- readLines(source_file, encoding = "latin1", warn = FALSE)
header_line <- 113L
data <- read.delim(
  textConnection(paste(lines[header_line:length(lines)], collapse = "\n")),
  sep = "\t",
  check.names = FALSE
)
# Drop the phantom trailing-tab column (spec §3/§4.2 step 4).
data <- data[, names(data) != "", drop = FALSE]

stopifnot(all(keep_columns %in% names(data)))
trimmed <- data[seq_len(n_rows), keep_columns, drop = FALSE]
trimmed[["time/s"]] <- trimmed[["time/s"]] - trimmed[["time/s"]][1]

# Sanity check: at least 20 rests at the default zero threshold, matching
# the state-classification rule this fixture is meant to exercise.
state <- ifelse(trimmed[["I/mA"]] == 0, "R", ifelse(trimmed[["I/mA"]] > 0, "charge", "discharge"))
starts <- c(FALSE, utils::head(state, -1) == "R" & state[-1] != "R")
rest <- cumsum(starts) + 1L
n_rests <- length(unique(rest))
cat(sprintf("Trimmed fixture covers %d rests over %d rows.\n", n_rests, nrow(trimmed)))
stopifnot(n_rests >= 20L)

dir.create(dirname(out_file), recursive = TRUE, showWarnings = FALSE)
write.table(
  trimmed,
  out_file,
  sep = "\t",
  row.names = FALSE,
  quote = FALSE,
  fileEncoding = "UTF-8",
  eol = "\n"
)
cat(sprintf("Wrote %s (%d rows x %d cols).\n", out_file, nrow(trimmed), ncol(trimmed)))
