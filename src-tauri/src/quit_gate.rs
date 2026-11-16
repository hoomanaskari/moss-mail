//! Native quit handshake. No timeout grants permission to discard renderer edits.
use std::sync::{Arc, Mutex};
use tauri::State;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Ask(String),
    Confirm(String),
    Focus,
    Finish { restart: bool },
}
#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    Idle,
    Asked { token: String, restart: bool },
    Confirming { token: String, restart: bool },
    Blocked { token: String, restart: bool },
    Cleared,
}
struct Gate {
    phase: Phase,
    serial: u64,
}
impl Default for Gate {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            serial: 0,
        }
    }
}
impl Gate {
    fn request(&mut self, restart: bool) -> Option<Effect> {
        match &self.phase {
            Phase::Cleared => None,
            Phase::Asked { token, restart } => {
                let effect = Effect::Confirm(token.clone());
                self.phase = Phase::Confirming {
                    token: token.clone(),
                    restart: *restart,
                };
                Some(effect)
            }
            Phase::Confirming { .. } => Some(Effect::Focus),
            Phase::Idle | Phase::Blocked { .. } => {
                self.serial += 1;
                let token = self.serial.to_string();
                self.phase = Phase::Asked {
                    token: token.clone(),
                    restart,
                };
                Some(Effect::Ask(token))
            }
        }
    }
    fn cancel(&mut self, token: &str) -> Option<Effect> {
        self.matching(token)?;
        self.phase = Phase::Idle;
        Some(Effect::Focus)
    }
    fn matching(&self, token: &str) -> Option<bool> {
        match &self.phase {
            Phase::Asked {
                token: current,
                restart,
            }
            | Phase::Confirming {
                token: current,
                restart,
            }
            | Phase::Blocked {
                token: current,
                restart,
            } if current == token => Some(*restart),
            _ => None,
        }
    }
    fn ready(&mut self, token: &str) -> Option<Effect> {
        let restart = self.matching(token)?;
        self.phase = Phase::Cleared;
        Some(Effect::Finish { restart })
    }
    fn blocked(&mut self, token: &str) -> Option<Effect> {
        let restart = self.matching(token)?;
        self.phase = Phase::Blocked {
            token: token.into(),
            restart,
        };
        Some(Effect::Focus)
    }
}

#[derive(Default)]
pub struct QuitGate(Mutex<Gate>);
pub struct Actions(pub Arc<dyn Fn(Effect) + Send + Sync>);
impl QuitGate {
    /// True means the native exit must be prevented, including emit failures.
    pub fn request(&self, restart: bool, actions: &Actions) -> bool {
        let effect = self.0.lock().unwrap().request(restart);
        if let Some(effect) = effect {
            (actions.0)(effect);
            true
        } else {
            false
        }
    }
    pub fn ready(&self, token: &str, actions: &Actions) -> bool {
        let effect = self.0.lock().unwrap().ready(token);
        if let Some(effect) = effect {
            (actions.0)(effect);
            true
        } else { false }
    }
    pub fn cancel(&self, token: &str, actions: &Actions) -> bool {
        let effect = self.0.lock().unwrap().cancel(token);
        if let Some(effect) = effect {
            (actions.0)(effect);
            true
        } else { false }
    }
    pub fn blocked(&self, token: &str, actions: &Actions) -> bool {
        let effect = self.0.lock().unwrap().blocked(token);
        if let Some(effect) = effect {
            (actions.0)(effect);
            true
        } else { false }
    }
}

#[tauri::command]
pub fn quit_ready(gate: State<'_, QuitGate>, actions: State<'_, Actions>, token: String) -> bool {
    gate.ready(&token, &actions)
}
#[tauri::command]
pub fn quit_blocked(gate: State<'_, QuitGate>, actions: State<'_, Actions>, token: String) -> bool {
    gate.blocked(&token, &actions)
}
#[tauri::command]
pub fn quit_cancel(gate: State<'_, QuitGate>, actions: State<'_, Actions>, token: String) -> bool {
    gate.cancel(&token, &actions)
}
#[tauri::command]
pub fn request_restart(gate: State<'_, QuitGate>, actions: State<'_, Actions>) {
    gate.request(true, &actions);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn asks_once_then_requires_an_explicit_native_confirmation() {
        let mut gate = Gate::default();
        assert_eq!(gate.request(false), Some(Effect::Ask("1".into())));
        assert_eq!(gate.request(false), Some(Effect::Confirm("1".into())));
        assert_eq!(gate.request(false), Some(Effect::Focus));
        assert_eq!(gate.blocked("1"), Some(Effect::Focus));
        assert_eq!(gate.request(false), Some(Effect::Ask("2".into())));
        assert_eq!(gate.ready("1"), None);
        assert_eq!(gate.blocked("1"), None);
        assert_eq!(gate.ready("2"), Some(Effect::Finish { restart: false }));
        assert_eq!(gate.request(false), None);
        assert_eq!(gate.ready("2"), None);
        assert_eq!(gate.blocked("2"), None);
    }
    #[test]
    fn restart_is_gated_before_calling_the_unpreventable_runtime_restart() {
        let mut gate = Gate::default();
        assert_eq!(gate.ready("1"), None);
        assert_eq!(gate.blocked("1"), None);
        assert_eq!(gate.request(true), Some(Effect::Ask("1".into())));
        assert_eq!(gate.request(false), Some(Effect::Confirm("1".into())));
        assert_eq!(gate.ready("1"), Some(Effect::Finish { restart: true }));
        assert_eq!(gate.request(true), None);
    }
    #[test]
    fn cancel_invalidates_a_late_renderer_ready() {
        let mut gate = Gate::default();
        gate.request(false);
        gate.request(false);
        assert_eq!(gate.cancel("wrong"), None);
        assert_eq!(gate.cancel("1"), Some(Effect::Focus));
        assert_eq!(gate.ready("1"), None);
        assert_eq!(gate.request(false), Some(Effect::Ask("2".into())));
        assert_eq!(gate.blocked("2"), Some(Effect::Focus));
        assert_eq!(gate.ready("2"), Some(Effect::Finish { restart: false }));
    }

    #[test]
    fn dispatch_releases_the_mutex_before_callbacks_and_ignores_stale_tokens() {
        let gate = Arc::new(QuitGate::default());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let observer = seen.clone();
        let checking = gate.clone();
        let actions = Actions(Arc::new(move |effect| {
            assert!(checking.0.try_lock().is_ok());
            observer.lock().unwrap().push(effect);
        }));
        gate.ready("stale", &actions);
        gate.blocked("stale", &actions);
        gate.cancel("stale", &actions);
        assert!(gate.request(false, &actions));
        gate.blocked("1", &actions);
        gate.cancel("1", &actions);
        assert!(gate.request(true, &actions));
        gate.ready("2", &actions);
        assert!(!gate.request(false, &actions));
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                Effect::Ask("1".into()),
                Effect::Focus,
                Effect::Focus,
                Effect::Ask("2".into()),
                Effect::Finish { restart: true }
            ]
        );
    }
}
