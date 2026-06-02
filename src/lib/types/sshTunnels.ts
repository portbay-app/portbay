export type SshAuthKind = "key" | "password" | "agent";
export type SshForwardKind = "local" | "reverse" | "socks";
export type SshTunnelState = "live" | "down" | "reconnecting";

export interface SshTunnelRuntimeStatus {
  id: string;
  /** Connection (host + auth) this forward rides on; used to open the file manager. */
  connectionId: string;
  name: string;
  sshHost: string;
  sshPort: number;
  sshUser: string;
  authKind: SshAuthKind;
  keyPath: string | null;
  localHost: string;
  localPort: number;
  remoteHost: string;
  remotePort: number;
  forwardKind: SshForwardKind;
  proxyJump: string | null;
  keepAlive: boolean;
  autoReconnect: boolean;
  state: SshTunnelState;
  running: boolean;
  startedAtMs: number | null;
  command: string;
}

export interface SaveSshTunnelInput {
  id?: string | null;
  name: string;
  sshHost: string;
  sshPort: number;
  sshUser: string;
  authKind: SshAuthKind;
  keyPath?: string | null;
  password?: string | null;
  localHost: string;
  localPort?: number | null;
  remoteHost: string;
  remotePort: number;
  forwardKind: SshForwardKind;
  proxyJump?: string | null;
  keepAlive: boolean;
  autoReconnect: boolean;
}

export interface OpenSshTunnelDatabaseInput {
  id: string;
  engine: "mysql" | "mariadb" | "postgres" | "redis" | "mongo" | "memcached";
}

/** Result of one remote command (deploy step). */
export interface StepResult {
  command: string;
  stdout: string;
  stderr: string;
  exitCode: number;
}

/** Result of one `ssh_exec_run` command: captured output + exit code. */
export interface ExecResult {
  stdout: string;
  stderr: string;
  /** Process exit code; `-1` if the server never reported one. */
  exitCode: number;
}

/** One remote file or directory, as returned by the SFTP file-manager commands. */
export interface SftpEntry {
  name: string;
  path: string;
  isDir: boolean;
  isSymlink: boolean;
  size: number;
  /** POSIX mode bits (e.g. 0o644), when the server reports them. */
  permissions: number | null;
  /** Modification time, seconds since the Unix epoch, when reported. */
  mtimeSecs: number | null;
}
