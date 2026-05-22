/**
 * Wire shape of `commands::metrics::SystemMetrics`. macOS-first today;
 * Linux/Windows port later (sysinfo handles cross-platform on its side).
 */

export interface CpuMetrics {
  /** 0..=100, aggregate across all cores. */
  total: number;
}

export interface MemoryMetrics {
  usedBytes: number;
  totalBytes: number;
}

export interface SystemMetrics {
  cpu: CpuMetrics;
  memory: MemoryMetrics;
}
