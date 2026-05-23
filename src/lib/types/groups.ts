/**
 * Wire shapes for `commands::groups`.
 */

export interface GroupView {
  id: string;
  name: string;
  projectIds: string[];
  /** Subset of `projectIds` that exist in the registry. */
  knownIds: string[];
  memberCount: number;
}

export interface GroupInput {
  id?: string;
  name: string;
  projectIds: string[];
}

export interface GroupPatch {
  name?: string;
  projectIds?: string[];
}

export interface GroupOpResult {
  projectId: string;
  ok: boolean;
  error: string | null;
}

export interface GroupOpReport {
  groupId: string;
  succeeded: number;
  failed: number;
  results: GroupOpResult[];
}
