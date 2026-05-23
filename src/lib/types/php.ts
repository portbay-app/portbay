/**
 * Wire shapes for the `commands::php` surface.
 */

export type PhpSource = "homebrew" | "system";

export interface PhpInstall {
  version: string;
  phpBin: string;
  phpFpmBin: string | null;
  phpIni: string | null;
  additionalIniDir: string | null;
  extensionDir: string | null;
  loadedExtensions: string[];
  source: PhpSource;
}

/** Xdebug modes accepted by the runtime — `off` deletes the env
 *  var entirely, anything else is set as-is. */
export const XDEBUG_MODES = [
  "off",
  "develop",
  "debug",
  "develop,debug",
  "profile",
  "trace",
  "coverage",
] as const;
export type XdebugMode = (typeof XDEBUG_MODES)[number];

/** Extensions PortBay knows how to surface "is Xdebug loaded?" etc. */
export const COMMON_EXTENSIONS = [
  "xdebug",
  "imagick",
  "redis",
  "mongodb",
  "swoole",
  "memcached",
  "gd",
  "mysqli",
  "pdo_mysql",
  "pdo_pgsql",
  "opcache",
  "apcu",
  "sodium",
] as const;
