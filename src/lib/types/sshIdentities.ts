import type { SshAuthKind } from "$lib/types/sshTunnels";

/** A reusable credential (user + key/agent/password method) shared across hosts. */
export interface SshIdentityView {
  id: string;
  name: string;
  sshUser: string;
  authKind: SshAuthKind;
  keyPath: string | null;
  /** How many connections borrow this identity. */
  connectionCount: number;
  /** Whether any connection borrows it (delete is blocked while true). */
  inUse: boolean;
}

export interface SaveSshIdentityInput {
  id?: string | null;
  name: string;
  sshUser: string;
  authKind: SshAuthKind;
  keyPath?: string | null;
}
