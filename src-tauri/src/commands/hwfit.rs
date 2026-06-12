//! Hardware profile for the AI page's model-fit recommender.
//!
//! Answers "which local models can this machine actually run well?" with one
//! cheap detection pass: chip name (sysctl brand string), total RAM, and a
//! unified-memory bandwidth lookup for Apple Silicon. Token throughput for
//! local LLM generation is memory-bandwidth-bound, so bandwidth ÷ active-weight
//! bytes gives a usable tokens/sec estimate — the scoring itself lives in the
//! frontend (`src/lib/hwfit.ts`) next to the catalog it scores.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareProfile {
    /// Marketing chip name, e.g. "Apple M2" / "Apple M4 Max".
    pub chip: String,
    pub total_ram_gb: f64,
    /// Memory a model can realistically claim. On Apple Silicon the GPU
    /// working set is capped below total RAM (macOS `iogpu.wired_limit_mb`
    /// default: ~2/3 of RAM up to 36 GB, ~3/4 above) — the same ceiling
    /// Ollama/llama.cpp run into via Metal's recommendedMaxWorkingSetSize.
    pub budget_gb: f64,
    /// Unified-memory bandwidth in GB/s. `None` when the chip isn't in the
    /// table AND no backstop applies (currently never — a backstop always
    /// fills in), kept optional so the UI degrades if that changes.
    pub bandwidth_gbps: Option<f64>,
    /// True when `bandwidth_gbps` is a conservative backstop rather than a
    /// table hit — the UI labels speed estimates as rough.
    pub estimated: bool,
}

/// Apple Silicon unified-memory bandwidth (GB/s), from Apple's published
/// specs. Matched as a substring of the lowercased brand string; entries are
/// ordered longest-variant-first per generation so "m4 max" wins over "m4".
/// Where a chip ships in two memory bins (M3 Max 300/400, M4 Max 410/546) the
/// table holds the full-width bin — core-count detection isn't worth the
/// precision for a fit badge.
const APPLE_BANDWIDTH: &[(&str, f64)] = &[
    ("m1 ultra", 800.0),
    ("m1 max", 400.0),
    ("m1 pro", 200.0),
    ("m2 ultra", 800.0),
    ("m2 max", 400.0),
    ("m2 pro", 200.0),
    ("m3 ultra", 800.0),
    ("m3 max", 400.0),
    ("m3 pro", 150.0),
    ("m4 max", 546.0),
    ("m4 pro", 273.0),
    ("m5 pro", 273.0),
    ("m1", 68.0),
    ("m2", 100.0),
    ("m3", 102.0),
    ("m4", 120.0),
    ("m5", 153.0),
];

/// Backstop for an Apple chip the table doesn't know yet (a future M6) —
/// assume at least base-chip class rather than hiding every estimate.
const APPLE_FALLBACK_GBPS: f64 = 120.0;
/// Non-Apple-Silicon backstop (Intel Mac, dev builds elsewhere): dual-channel
/// DDR4/DDR5 territory, deliberately conservative.
const CPU_FALLBACK_GBPS: f64 = 60.0;

fn chip_name() -> String {
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
        {
            let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !name.is_empty() {
                return name;
            }
        }
    }
    let sys = sysinfo::System::new_with_specifics(
        sysinfo::RefreshKind::new().with_cpu(sysinfo::CpuRefreshKind::new()),
    );
    sys.cpus()
        .first()
        .map(|c| c.brand().trim().to_string())
        .unwrap_or_default()
}

fn lookup_bandwidth(chip: &str) -> (Option<f64>, bool) {
    let lower = chip.to_lowercase();
    for (key, gbps) in APPLE_BANDWIDTH {
        if lower.contains(key) {
            return (Some(*gbps), false);
        }
    }
    if lower.contains("apple") {
        return (Some(APPLE_FALLBACK_GBPS), true);
    }
    (Some(CPU_FALLBACK_GBPS), true)
}

#[tauri::command]
pub fn hardware_profile() -> HardwareProfile {
    let chip = chip_name();
    let sys = sysinfo::System::new_with_specifics(
        sysinfo::RefreshKind::new().with_memory(sysinfo::MemoryRefreshKind::new().with_ram()),
    );
    let total_ram_gb = sys.total_memory() as f64 / 1_073_741_824.0;
    let budget_gb = if total_ram_gb > 36.0 {
        total_ram_gb * 0.75
    } else {
        total_ram_gb * (2.0 / 3.0)
    };
    let (bandwidth_gbps, estimated) = lookup_bandwidth(&chip);
    HardwareProfile {
        chip,
        total_ram_gb,
        budget_gb,
        bandwidth_gbps,
        estimated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longer_variant_wins_over_base_chip() {
        assert_eq!(lookup_bandwidth("Apple M4 Max"), (Some(546.0), false));
        assert_eq!(lookup_bandwidth("Apple M4 Pro"), (Some(273.0), false));
        assert_eq!(lookup_bandwidth("Apple M4"), (Some(120.0), false));
        assert_eq!(lookup_bandwidth("Apple M2"), (Some(100.0), false));
    }

    #[test]
    fn unknown_apple_chip_gets_estimated_backstop() {
        let (bw, estimated) = lookup_bandwidth("Apple M9 Galactic");
        assert_eq!(bw, Some(APPLE_FALLBACK_GBPS));
        assert!(estimated);
    }

    #[test]
    fn non_apple_gets_cpu_backstop() {
        let (bw, estimated) = lookup_bandwidth("Intel(R) Core(TM) i9-9980HK");
        assert_eq!(bw, Some(CPU_FALLBACK_GBPS));
        assert!(estimated);
    }
}
