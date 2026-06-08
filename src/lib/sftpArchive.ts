/**
 * Remote archive handling: recognize archives by name and extract them **on
 * the host** over the existing exec channel (`ssh_exec_run`) — so a zip
 * uploaded via SFTP can be unpacked in place without a terminal round-trip.
 *
 * Extraction always creates the destination directory first (`mkdir -p`), and
 * every path is single-quote shell-escaped, so spaces and metacharacters in
 * file names can't break out of the command.
 */
import { posixBasename } from "$lib/sftp";
import { sshExecRun } from "$lib/sshExec";
import { errorBus } from "$lib/stores/errors.svelte";

export type ArchiveKind =
  | "zip"
  | "tar"
  | "tgz"
  | "tbz2"
  | "txz"
  | "gz"
  | "bz2"
  | "xz"
  | "7z"
  | "rar";

// Order matters: compound extensions (`.tar.gz`) must match before their
// single-suffix cousins (`.gz`).
const KINDS: { re: RegExp; kind: ArchiveKind }[] = [
  { re: /\.zip$/i, kind: "zip" },
  { re: /\.(tar\.gz|tgz)$/i, kind: "tgz" },
  { re: /\.(tar\.bz2|tbz2?)$/i, kind: "tbz2" },
  { re: /\.(tar\.xz|txz)$/i, kind: "txz" },
  { re: /\.tar$/i, kind: "tar" },
  { re: /\.gz$/i, kind: "gz" },
  { re: /\.bz2$/i, kind: "bz2" },
  { re: /\.xz$/i, kind: "xz" },
  { re: /\.7z$/i, kind: "7z" },
  { re: /\.rar$/i, kind: "rar" },
];

/** The tool each kind needs on the server (for the not-installed message). */
const TOOL: Record<ArchiveKind, string> = {
  zip: "unzip",
  tar: "tar",
  tgz: "tar",
  tbz2: "tar",
  txz: "tar",
  gz: "gzip",
  bz2: "bzip2",
  xz: "xz",
  "7z": "7z",
  rar: "unrar",
};

export function archiveKind(name: string): ArchiveKind | null {
  for (const k of KINDS) if (k.re.test(name)) return k.kind;
  return null;
}

export function isArchive(name: string): boolean {
  return archiveKind(name) !== null;
}

/** Archive name minus its archive extension — the default extract folder. */
export function archiveStem(name: string): string {
  for (const k of KINDS) {
    if (k.re.test(name)) {
      const stem = name.replace(k.re, "");
      return stem || name;
    }
  }
  return name;
}

/** POSIX shell single-quote escaping: `'` → `'\''`. */
function shq(p: string): string {
  return `'${p.replaceAll("'", "'\\''")}'`;
}

/** The extraction command for one archive into `destDir` (created if absent). */
export function extractCommand(remotePath: string, destDir: string, kind: ArchiveKind): string {
  const f = shq(remotePath);
  const d = shq(destDir);
  const mk = `mkdir -p ${d} && `;
  // Single-file compressors decompress to <dest>/<name minus extension>.
  const single = (tool: string) =>
    `${mk}${tool} -dc ${f} > ${shq(`${destDir}/${archiveStem(posixBasename(remotePath))}`)}`;
  switch (kind) {
    case "zip":
      return `${mk}unzip -o ${f} -d ${d}`;
    case "tar":
    case "tgz":
    case "tbz2":
    case "txz":
      // Modern tar auto-detects the compression from the file.
      return `${mk}tar -xf ${f} -C ${d}`;
    case "gz":
      return single("gzip");
    case "bz2":
      return single("bzip2");
    case "xz":
      return single("xz");
    case "7z":
      return `${mk}7z x -y -o${d} ${f}`;
    case "rar":
      return `${mk}unrar x -o+ ${f} ${shq(`${destDir}/`)}`;
  }
}

/**
 * Extract a remote archive into `destDir` on the same host. Pushes a success
 * toast on completion and a detailed failure toast (including a "tool not
 * installed" hint on exit 127) otherwise; throws on failure so callers can
 * skip their refresh.
 */
export async function extractArchive(
  connectionId: string,
  remotePath: string,
  destDir: string,
): Promise<void> {
  const name = posixBasename(remotePath);
  const kind = archiveKind(name);
  if (!kind) throw new Error(`not a recognized archive: ${name}`);

  // sshExecRun toasts transport-level failures itself and rejects.
  const res = await sshExecRun(connectionId, extractCommand(remotePath, destDir, kind));
  if (res.exitCode !== 0) {
    const missingTool = res.exitCode === 127 || /command not found/i.test(res.stderr);
    errorBus.push({
      code: "SFTP_EXTRACT_FAILED",
      category: "infrastructure",
      whatHappened: missingTool
        ? `“${TOOL[kind]}” isn't installed on the server.`
        : `Couldn't extract “${name}”.`,
      whyItMatters: missingTool
        ? `Extracting ${name} needs ${TOOL[kind]} on the remote host.`
        : "The archive wasn't unpacked — the destination may be missing or partial.",
      whoCausedIt: "system",
      actions: [],
      details: (res.stderr || res.stdout).trim() || undefined,
    });
    throw new Error(`extract failed (exit ${res.exitCode})`);
  }

  errorBus.push({
    code: "SFTP_EXTRACTED",
    category: "infrastructure",
    whatHappened: `Extracted “${name}”.`,
    whyItMatters: `Contents are in ${destDir}.`,
    whoCausedIt: "system",
    severity: "success",
    actions: [],
  });
}

/**
 * The compression command: zip `names` (entries directly inside `baseDir`)
 * into `baseDir/zipName`. Runs from `baseDir` so the archive holds relative
 * paths, not the absolute server layout. `-y` keeps symlinks as links instead
 * of following them (a linked dir could recurse or balloon the archive).
 */
export function compressCommand(baseDir: string, names: readonly string[], zipName: string): string {
  const rels = names.map((n) => shq(n)).join(" ");
  return `cd ${shq(baseDir)} && zip -r -q -y ${shq(zipName)} ${rels}`;
}

/**
 * Zip remote entries (all directly inside `baseDir`) into `baseDir/zipName`
 * on the host. Same toast contract as extractArchive: success toast on
 * completion, detailed failure toast (with a "zip isn't installed" hint on
 * exit 127), throws on failure so callers can skip their refresh.
 */
export async function compressEntries(
  connectionId: string,
  baseDir: string,
  names: readonly string[],
  zipName: string,
): Promise<void> {
  if (names.length === 0) throw new Error("nothing to compress");

  const res = await sshExecRun(connectionId, compressCommand(baseDir, names, zipName));
  if (res.exitCode !== 0) {
    const missingTool = res.exitCode === 127 || /command not found/i.test(res.stderr);
    errorBus.push({
      code: "SFTP_COMPRESS_FAILED",
      category: "infrastructure",
      whatHappened: missingTool
        ? "“zip” isn't installed on the server."
        : `Couldn't compress ${names.length === 1 ? `“${names[0]}”` : `${names.length} items`}.`,
      whyItMatters: missingTool
        ? "Creating the archive needs the zip tool on the remote host."
        : "The archive wasn't created — or may be partial. Check the folder.",
      whoCausedIt: "system",
      actions: [],
      details: (res.stderr || res.stdout).trim() || undefined,
    });
    throw new Error(`compress failed (exit ${res.exitCode})`);
  }

  errorBus.push({
    code: "SFTP_COMPRESSED",
    category: "infrastructure",
    whatHappened: `Created “${zipName}”.`,
    whyItMatters: `The archive is in ${baseDir}.`,
    whoCausedIt: "system",
    severity: "success",
    actions: [],
  });
}
