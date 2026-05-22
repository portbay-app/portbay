/**
 * Search store — the top bar's search input mutates this; every consumer
 * (projects table for Phase 2; services / domains / logs later) reads it.
 *
 * Kept dead-simple — a string. Filtering logic lives in the consumer
 * because each surface has different match semantics (project name +
 * domain ≠ log line text).
 */

function createSearchStore() {
  let query = $state<string>("");
  return {
    get value() {
      return query;
    },
    set(next: string) {
      query = next;
    },
    clear() {
      query = "";
    },
  };
}

export const search = createSearchStore();
