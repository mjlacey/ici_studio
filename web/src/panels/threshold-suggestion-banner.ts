// §7.1 / §16 decision #7: if the current `state_threshold` misclassifies a
// group entirely (no rests, or no active samples -- typically because the
// instrument's "off" current isn't exactly zero), `runSegmentation`'s
// summary carries a suggested threshold from `split_current_levels()`. This
// is a blocking banner, not a silent auto-apply: the wrong threshold can
// otherwise make the raw time series/regression diagnostic look "broken"
// with no indication why.

import { activeDataset, store, updateDataset } from "../state";

export function mountThresholdSuggestionBanner(container: HTMLElement): void {
  render();
  store.subscribe(render);

  function render(): void {
    const dataset = activeDataset(store.get());
    const suggestions = dataset?.segmentation?.thresholdSuggestions ?? [];
    if (!dataset || suggestions.length === 0) {
      container.innerHTML = "";
      return;
    }

    // stateThreshold is a single global value -- when multiple groups each
    // suggest a (possibly different) threshold, the max is the safest single
    // choice: it clears the noise floor for every affected group.
    const suggested = Math.max(...suggestions.map((s) => s.suggestedThreshold));
    const groupNote = suggestions.length > 1 ? ` (affects groups: ${suggestions.map((s) => s.groupId).join(", ")})` : "";

    container.innerHTML = `
      <div class="issue-block threshold-suggestion-banner">
        No rests or no active samples were detected at the current state threshold${groupNote} -- the instrument's "off" current may be non-zero rather than exactly 0.
        <button type="button" class="btn-small" data-apply-threshold>Use suggested threshold: ${suggested.toPrecision(3)} A</button>
      </div>
    `;

    container.querySelector<HTMLButtonElement>("[data-apply-threshold]")!.addEventListener("click", () => {
      updateDataset(dataset.id, (d) => ({
        ...d,
        stageAConfig: { ...d.stageAConfig, stateThreshold: suggested },
      }));
    });
  }
}
