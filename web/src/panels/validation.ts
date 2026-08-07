// §5.3: validation relaxed from R's hard errors to warnings-with-actions,
// except duplicate mapping and non-numeric mapped columns, which stay hard
// blocks (matching R).

import { REQUIRED_FIELDS, type Dataset, type ValidationIssue } from "../types";
import type { DataWorkerClient } from "../worker/client";

export async function computeValidation(dataset: Dataset, worker: DataWorkerClient): Promise<ValidationIssue[]> {
  const issues: ValidationIssue[] = [];
  if (!dataset.columns || !dataset.report) return issues;

  // Duplicate column assignment -> hard block.
  const byColumn = new Map<string, string[]>();
  for (const field of REQUIRED_FIELDS) {
    const col = dataset.mapping[field].column;
    if (!col) continue;
    byColumn.set(col, [...(byColumn.get(col) ?? []), field]);
  }
  for (const [col, fields] of byColumn) {
    if (fields.length > 1) {
      issues.push({
        kind: "duplicateColumn",
        severity: "block",
        message: `'${col}' is mapped to more than one field (${fields.join(", ")}).`,
        column: col,
      });
    }
  }

  // Non-numeric mapped column -> hard block.
  for (const field of REQUIRED_FIELDS) {
    const colName = dataset.mapping[field].column;
    if (!colName) continue;
    const colInfo = dataset.columns.find((c) => c.name === colName);
    if (colInfo && !colInfo.isNumeric) {
      issues.push({
        kind: "nonNumericColumn",
        severity: "block",
        message: `'${colName}' (mapped to ${field}) is not numeric.`,
        column: colName,
      });
    }
  }

  // Non-finite values in a mapped column -> warning, default action "drop".
  for (const field of REQUIRED_FIELDS) {
    const colName = dataset.mapping[field].column;
    if (!colName) continue;
    const colInfo = dataset.columns.find((c) => c.name === colName);
    if (!colInfo?.isNumeric) continue;
    const stats = await worker.getColumnStats(dataset.id, colName);
    if (!stats) continue;
    const nonFiniteCount = dataset.report.nRows - stats.nFinite;
    if (nonFiniteCount > 0) {
      issues.push({
        kind: "nonFinite",
        severity: "warning",
        message: `${nonFiniteCount} non-finite value(s) in '${colName}' (mapped to ${field}).`,
        column: colName,
        count: nonFiniteCount,
        action: "dropRows",
      });
    }
  }

  // Non-monotonic time -> warning; blocks Stage A once it exists (milestone 8).
  const timeCol = dataset.mapping.time.column;
  if (timeCol) {
    const result = await worker.checkTimeMonotonic(dataset.id, timeCol);
    if (!result.isMonotonic) {
      issues.push({
        kind: "nonMonotonicTime",
        severity: "warning",
        message: `Time is not monotonically increasing (first offending row: ${result.firstOffendingRow}).`,
        column: timeCol,
        firstRowIndex: result.firstOffendingRow ?? undefined,
        action: "sortByTime",
      });
    }
  }

  return issues;
}

export function hasBlockingIssues(issues: ValidationIssue[]): boolean {
  return issues.some((i) => i.severity === "block");
}
