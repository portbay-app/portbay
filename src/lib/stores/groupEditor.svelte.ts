/**
 * Open/close state for the group editor modal. Two modes: create (new
 * group) and edit (existing group). Owned outside the modal so the
 * sidebar's "New group" button and the filtered view's "Edit group"
 * button can both drive the same instance.
 */
import type { GroupView } from "$lib/types/groups";

type Mode = { kind: "closed" } | { kind: "create" } | { kind: "edit"; group: GroupView };

function createGroupEditorStore() {
  let mode = $state<Mode>({ kind: "closed" });
  return {
    get mode() {
      return mode;
    },
    get isOpen() {
      return mode.kind !== "closed";
    },
    create() {
      mode = { kind: "create" };
    },
    edit(group: GroupView) {
      mode = { kind: "edit", group };
    },
    close() {
      mode = { kind: "closed" };
    },
  };
}

export const groupEditor = createGroupEditorStore();
