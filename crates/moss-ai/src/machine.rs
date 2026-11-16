//! What this computer is: its chip and memory, read from the system. Moss uses it to say
//! which models fit and which one to recommend.

use serde::Serialize;

const GIB: u64 = 1 << 30;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Machine {
    /// As the system names it, e.g. "Apple M4 Max".
    pub chip: String,
    /// Total memory in bytes.
    pub memory: u64,
}

impl Machine {
    /// Reads the real machine.
    pub fn detect() -> Machine {
        use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
        let system = System::new_with_specifics(
            RefreshKind::nothing()
                .with_memory(MemoryRefreshKind::nothing().with_ram())
                .with_cpu(CpuRefreshKind::nothing()),
        );
        let chip = system
            .cpus()
            .first()
            .map(|c| c.brand().trim().to_string())
            .unwrap_or_default();
        Machine {
            chip,
            memory: system.total_memory(),
        }
    }

    /// Whole gigabytes of memory, as Apple sells them: 36 for a 36 GB Mac.
    pub fn memory_gb(&self) -> u64 {
        (self.memory + GIB / 2) / GIB
    }

    /// For Settings: "M4 Max, 36 GB memory".
    pub fn label(&self) -> String {
        let chip = self.chip.trim();
        let chip = chip.strip_prefix("Apple ").unwrap_or(chip);
        let memory = format!("{} GB memory", self.memory_gb());
        if chip.is_empty() {
            memory
        } else {
            format!("{chip}, {memory}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mac(chip: &str, gb: u64) -> Machine {
        Machine {
            chip: chip.into(),
            memory: gb * GIB,
        }
    }

    #[test]
    fn names_the_chip_and_memory_the_way_apple_does() {
        assert_eq!(mac("Apple M4 Max", 36).label(), "M4 Max, 36 GB memory");
        assert_eq!(mac("Apple M1", 8).label(), "M1, 8 GB memory");
        assert_eq!(
            mac("Intel(R) Core(TM) i7-8700", 16).label(),
            "Intel(R) Core(TM) i7-8700, 16 GB memory"
        );
        assert_eq!(mac("  ", 16).label(), "16 GB memory");
    }

    #[test]
    fn memory_rounds_to_whole_gigabytes() {
        assert_eq!(mac("x", 16).memory_gb(), 16);
        // The system can report a little less than the memory sold, never round down to 15.
        assert_eq!(
            Machine {
                chip: "x".into(),
                memory: 16 * GIB - 300 * (1 << 20)
            }
            .memory_gb(),
            16
        );
        assert_eq!(
            Machine {
                chip: "x".into(),
                memory: 0
            }
            .memory_gb(),
            0
        );
    }

    #[test]
    fn reads_this_machine() {
        let m = Machine::detect();
        assert!(m.memory > GIB, "{m:?}");
        assert!(!m.chip.trim().is_empty(), "{m:?}");
    }
}
