//! When a big download should wait: on battery below 20%, or on a phone's hotspot, where
//! data costs money. Read from macOS's own tools, so nothing needs extra permissions.

use std::time::Duration;

use serde::Serialize;

/// Why a download is waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Pause {
    /// On battery, below [`LOW_BATTERY`] percent.
    Battery,
    /// Online through a phone's hotspot.
    Hotspot,
}

/// Downloads wait on battery below this percentage.
pub const LOW_BATTERY: u32 = 20;

/// From `pmset -g batt`: whether the Mac runs on a battery that's low. A Mac without a
/// battery, or on power, never is.
pub fn low_battery(pmset: &str) -> bool {
    if !pmset.contains("'Battery Power'") {
        return false;
    }
    let percent = pmset.split(|c: char| c.is_whitespace() || c == ';').find_map(|w| w.strip_suffix('%')?.parse::<u32>().ok());
    percent.is_some_and(|p| p < LOW_BATTERY)
}

/// From `route -n get default`: whether the way out is an iPhone's Personal Hotspot, which
/// always hands out addresses in 172.20.10.0/28.
pub fn on_hotspot(route: &str) -> bool {
    route.lines().filter_map(|l| l.trim().strip_prefix("gateway:")).any(|g| {
        let g = g.trim();
        g.strip_prefix("172.20.10.").and_then(|last| last.parse::<u8>().ok()).is_some_and(|n| n < 16)
    })
}

/// From `pmset -g batt`: whether the Mac runs on its battery now, at any level. A Mac
/// without a battery never does.
pub fn on_battery(pmset: &str) -> bool {
    pmset.contains("'Battery Power'")
}

/// Where the Mac's power comes from, asked afresh each time. Background AI work (indexing
/// mail by meaning, summaries nobody asked for yet) waits while it's the battery; what you
/// ask for runs anyway. The app uses [`MacPower`]; tests pass a fake.
pub trait PowerSource: Send + Sync {
    fn on_battery(&self) -> bool;
}

/// This Mac, through `pmset`.
pub struct MacPower;

impl PowerSource for MacPower {
    fn on_battery(&self) -> bool {
        on_battery(&run("pmset", &["-g", "batt"]))
    }
}

/// How long `pmset` or `route` may take to answer before Moss stops waiting for it.
const ASK_WAIT: Duration = Duration::from_secs(2);

fn run(program: &str, args: &[&str]) -> String {
    run_within(program, args, ASK_WAIT)
}

/// What `program` prints, or nothing when it can't run or doesn't finish within `wait`.
fn run_within(program: &str, args: &[&str], wait: Duration) -> String {
    let child = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return String::new() };
    let deadline = std::time::Instant::now() + wait;
    while matches!(child.try_wait(), Ok(None)) {
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return String::new();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut out = String::new();
    let _ = std::io::Read::read_to_string(&mut child.stdout.take().expect("stdout is piped"), &mut out);
    out
}

/// Whether a download should wait, from what `pmset -g batt` and `route -n get default` say.
pub fn reasons(pmset: &str, route: &str) -> Option<Pause> {
    if low_battery(pmset) {
        return Some(Pause::Battery);
    }
    on_hotspot(route).then_some(Pause::Hotspot)
}

/// Whether a download should wait now, and why.
pub fn conditions() -> Option<Pause> {
    reasons(&run("pmset", &["-g", "batt"]), &run("route", &["-n", "get", "default"]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_low_battery_is_only_low_when_running_on_it() {
        let on_battery = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=4653155)\t18%; discharging; 1:02 remaining present: true\n";
        assert!(low_battery(on_battery));
        assert!(!low_battery(&on_battery.replace("18%", "20%")));
        assert!(!low_battery(&on_battery.replace("'Battery Power'", "'AC Power'")));
        assert!(!low_battery("Now drawing from 'AC Power'\n"));
        assert!(!low_battery("Now drawing from 'Battery Power'\n"));
    }

    #[test]
    fn a_phone_hotspot_is_known_by_its_gateway() {
        let route = |gw: &str| format!("   route to: default\ndestination: default\n    gateway: {gw}\n  interface: en0\n");
        assert!(on_hotspot(&route("172.20.10.1")));
        assert!(!on_hotspot(&route("172.20.10.17")));
        assert!(!on_hotspot(&route("192.168.1.1")));
        assert!(!on_hotspot(&route("172.20.10.x")));
        assert!(!on_hotspot(""));
    }

    #[test]
    fn battery_comes_before_the_hotspot() {
        let low = "Now drawing from 'Battery Power'\n -InternalBattery-0\t5%; discharging;";
        let hotspot = "gateway: 172.20.10.1";
        assert_eq!(reasons(low, hotspot), Some(Pause::Battery));
        assert_eq!(reasons("", hotspot), Some(Pause::Hotspot));
        assert_eq!(reasons("", ""), None);
    }

    #[test]
    fn this_mac_can_be_asked() {
        // Whatever the answer, asking works and never panics.
        let _ = conditions();
        let _ = MacPower.on_battery();
        assert_eq!(run("/nonexistent/tool", &[]), "");
        assert_eq!(run("/bin/echo", &["hi"]), "hi\n");
    }

    #[test]
    fn a_tool_that_never_answers_is_not_waited_for() {
        let started = std::time::Instant::now();
        assert_eq!(run_within("/bin/sleep", &["5"], Duration::from_millis(50)), "");
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(ASK_WAIT, Duration::from_secs(2));
    }

    #[test]
    fn on_battery_is_any_level_of_it() {
        assert!(on_battery("Now drawing from 'Battery Power'\n -InternalBattery-0\t95%; discharging;"));
        assert!(!on_battery("Now drawing from 'AC Power'\n -InternalBattery-0\t95%; charged;"));
        assert!(!on_battery(""), "a Mac without a battery, or pmset not answering");
    }
}
