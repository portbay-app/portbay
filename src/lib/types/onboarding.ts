/**
 * Wire shapes for the `commands::onboarding` surface.
 */
import type { ProjectView } from "./projects";

export interface OnboardingStatus {
  onboarded: boolean;
  registryEmpty: boolean;
}

/** One of the five scaffolder templates exposed by the gallery. */
export type ScaffoldKind = "nextjs" | "vite" | "astro" | "laravel" | "php";

/** Streamed event from `scaffold_template`. */
export type ScaffoldEvent =
  | { kind: "log"; line: string }
  | { kind: "done"; projectId: string };

export interface TemplateEntry {
  kind: ScaffoldKind;
  name: string;
  description: string;
  /** Icon name from the `Icon` atom's set. */
  icon: string;
  /** Default folder name suggested in the picker. */
  defaultName: string;
  /** True when the upstream scaffolder needs a binary on PATH. */
  requiresBinary?: string;
}

export const TEMPLATES: TemplateEntry[] = [
  {
    kind: "nextjs",
    name: "Next.js",
    description: "React + App Router, Tailwind, TypeScript",
    icon: "package",
    defaultName: "my-next-app",
    requiresBinary: "pnpm",
  },
  {
    kind: "vite",
    name: "Vite",
    description: "Vanilla TypeScript, fast HMR, zero config",
    icon: "zap",
    defaultName: "my-vite-app",
    requiresBinary: "pnpm",
  },
  {
    kind: "astro",
    name: "Astro",
    description: "Content-driven, islands architecture, MDX-ready",
    icon: "star",
    defaultName: "my-astro-site",
    requiresBinary: "pnpm",
  },
  {
    kind: "laravel",
    name: "Laravel",
    description: "PHP, MVC, Artisan CLI, Composer dependencies",
    icon: "server",
    defaultName: "my-laravel-app",
    requiresBinary: "composer",
  },
  {
    kind: "php",
    name: "Plain PHP",
    description: "Single index.php, Caddy-served, no framework",
    icon: "file-code",
    defaultName: "my-php-site",
  },
];
