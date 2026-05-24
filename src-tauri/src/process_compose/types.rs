use serde::{Deserialize, Serialize};

/// Mirrors a row of Process Compose's `GET /processes` response.
///
/// Field names match PC's wire format exactly so we can decode directly.
/// Variants we don't care about today are still included as optional /
/// untyped so the deserialiser doesn't fail on future PC versions.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Process {
    pub name: String,
    pub namespace: String,
    pub status: String,
    #[serde(default)]
    pub is_running: bool,
    /// Process Compose reports the readiness probe's *last observed* state,
    /// not the *current* state. After a process dies, `is_ready` stays at
    /// its last-known value — see `Process::is_serving` and the spike
    /// report `claudedocs/spike-process-compose.md` § Quirk 1.
    #[serde(default)]
    pub is_ready: String,
    #[serde(default)]
    pub has_ready_probe: bool,
    #[serde(default)]
    pub pid: u32,
    #[serde(default)]
    pub exit_code: i32,
    #[serde(default)]
    pub restarts: u32,
    #[serde(default)]
    pub mem: u64,
    #[serde(default)]
    pub cpu: f64,
    #[serde(default)]
    pub age: u64,
}

impl Process {
    /// PortBay's authoritative "actually serving" predicate.
    ///
    /// Process Compose's `is_ready` field is *stale* after termination
    /// (see spike report § Quirk 1). We derive truth from `is_running`
    /// AND the readiness flag, never from readiness alone.
    pub fn is_serving(&self) -> bool {
        if !self.is_running {
            return false;
        }
        if !self.has_ready_probe {
            // No readiness probe configured → trust is_running.
            return true;
        }
        self.is_ready == "Ready"
    }

    /// PortBay's status taxonomy (`ASSESSMENT_AND_PLAN.md` §5.3) derived
    /// from PC's raw fields. Maps to GUI badges + CLI colors.
    ///
    /// Signal-exit handling: any exit code in the 128..=192 range comes
    /// from a UNIX signal (128 + signal number) — SIGINT/SIGTERM/SIGKILL
    /// are normal stop paths, not crashes. The runtime also reports -1
    /// when PC kills the child during a clean shutdown. We treat all of
    /// those as Stopped so clicking "Stop" never paints the row red.
    /// True crashes have small positive exit codes from the program
    /// itself (Node's `exit(1)`, panic, etc.).
    pub fn portbay_status(&self) -> ProjectStatus {
        match (self.is_running, self.status.as_str(), self.has_ready_probe) {
            (false, "Pending", _) => ProjectStatus::Stopped,
            (false, "Completed", _) if self.exit_code == 0 => ProjectStatus::Stopped,
            (false, _, _) if is_signal_exit(self.exit_code) => ProjectStatus::Stopped,
            (false, _, _) if self.exit_code != 0 && self.exit_code != -1 => ProjectStatus::Crashed,
            (false, _, _) => ProjectStatus::Stopped,
            (true, _, false) => ProjectStatus::Running,
            (true, _, true) if self.is_ready == "Ready" => ProjectStatus::Running,
            (true, _, true) if self.is_ready == "Starting" || self.is_ready.is_empty() => {
                ProjectStatus::Starting
            }
            (true, _, true) => ProjectStatus::Unhealthy,
        }
    }
}

/// True when the exit code looks like a signal-induced exit on UNIX.
/// 128..=192 covers signals 0..=64; we also accept the raw small
/// values for SIGINT/SIGTERM/SIGKILL that some runtimes report
/// directly (Node, Python).
fn is_signal_exit(code: i32) -> bool {
    matches!(code, 130 | 137 | 143) || (128..=192).contains(&code)
}

/// PortBay-side status taxonomy. Lives here instead of the registry so the
/// registry stays purely declarative (it describes *what should be*, not
/// *what is*).
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectStatus {
    Stopped,
    Starting,
    Running,
    Unhealthy,
    Crashed,
    /// Couldn't start because something else holds the port. Reported by
    /// the pre-flight conflict check, not by PC directly.
    PortConflict,
}

/// Shape of Process Compose's `GET /processes` response.
#[derive(Debug, Deserialize)]
pub(crate) struct ProcessesEnvelope {
    pub data: Vec<Process>,
}

/// Shape of Process Compose's `GET /process/logs/{name}/{offset}/{limit}`.
#[derive(Debug, Deserialize)]
pub struct LogsResponse {
    pub logs: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(running: bool, ready: &str, has_probe: bool, status: &str, exit: i32) -> Process {
        Process {
            name: "x".into(),
            namespace: "default".into(),
            status: status.into(),
            is_running: running,
            is_ready: ready.into(),
            has_ready_probe: has_probe,
            pid: 0,
            exit_code: exit,
            restarts: 0,
            mem: 0,
            cpu: 0.0,
            age: 0,
        }
    }

    #[test]
    fn is_serving_requires_is_running_first() {
        // The Quirk 1 case from the spike: process is dead but PC's is_ready
        // hasn't been refreshed. Must NOT report serving.
        let p = proc(false, "Ready", true, "Completed", -1);
        assert!(!p.is_serving());
    }

    #[test]
    fn is_serving_trusts_is_running_when_no_probe() {
        let p = proc(true, "-", false, "Running", 0);
        assert!(p.is_serving());
    }

    #[test]
    fn is_serving_requires_ready_when_probe_present() {
        let p = proc(true, "Starting", true, "Running", 0);
        assert!(!p.is_serving());
        let p = proc(true, "Ready", true, "Running", 0);
        assert!(p.is_serving());
    }

    #[test]
    fn portbay_status_taxonomy() {
        assert_eq!(
            proc(true, "Ready", true, "Running", 0).portbay_status(),
            ProjectStatus::Running
        );
        assert_eq!(
            proc(true, "-", false, "Running", 0).portbay_status(),
            ProjectStatus::Running
        );
        assert_eq!(
            proc(true, "Starting", true, "Running", 0).portbay_status(),
            ProjectStatus::Starting
        );
        // Unhealthy: running, probe present, not Ready, not Starting.
        assert_eq!(
            proc(true, "NotReady", true, "Running", 0).portbay_status(),
            ProjectStatus::Unhealthy
        );
        assert_eq!(
            proc(false, "Completed", true, "Completed", 0).portbay_status(),
            ProjectStatus::Stopped
        );
        // Crashed: exited with a real non-zero, non-signal exit code.
        assert_eq!(
            proc(false, "-", false, "Completed", 1).portbay_status(),
            ProjectStatus::Crashed
        );
        // Signal exit (-1 from SIGTERM/SIGKILL) is "stopped", not "crashed".
        assert_eq!(
            proc(false, "-", false, "Completed", -1).portbay_status(),
            ProjectStatus::Stopped
        );
    }

    #[test]
    fn processes_envelope_decodes() {
        let json = r#"{
            "data": [{
                "name": "ping",
                "namespace": "default",
                "status": "Running",
                "is_running": true,
                "is_ready": "Ready",
                "has_ready_probe": true,
                "pid": 12345,
                "exit_code": 0,
                "restarts": 0,
                "mem": 25575424,
                "cpu": 0.0,
                "age": 4366498083
            }]
        }"#;
        let env: ProcessesEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(env.data.len(), 1);
        assert_eq!(env.data[0].name, "ping");
        assert!(env.data[0].is_serving());
    }
}
