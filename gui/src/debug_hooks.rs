//! Developer hooks: `MUSICWASP_*` variables that open dialogs, seed state or run
//! actions for screenshots and tests.
//!
//! Every hook is inert unless `MUSICWASP_DEBUG=1` is set. A normal launch never
//! reads them, so a stray or inherited variable cannot trigger a tag write, an
//! undo, a scan or a restore.

const FLAG: &str = "MUSICWASP_DEBUG";

/// True when hooks are enabled for this process.
pub fn enabled() -> bool {
    std::env::var(FLAG).is_ok_and(|value| value == "1")
}

/// Value of a hook variable, or `None` unless hooks are enabled.
pub fn var(name: &str) -> Option<String> {
    gated(enabled(), name)
}

/// True when a hook variable is set (to any value) and hooks are enabled.
pub fn is_set(name: &str) -> bool {
    var(name).is_some()
}

fn gated(on: bool, name: &str) -> Option<String> {
    if on {
        std::env::var(name).ok()
    } else {
        None
    }
}

/// Log a notice so a session with hooks enabled is visible in the terminal.
pub fn announce() {
    if enabled() {
        eprintln!("MusicWasp: debug hooks enabled ({FLAG}=1)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_values_are_hidden_while_disabled() {
        std::env::set_var("MUSICWASP_GATE_PROBE", "1");
        assert_eq!(gated(false, "MUSICWASP_GATE_PROBE"), None);
        assert_eq!(gated(true, "MUSICWASP_GATE_PROBE").as_deref(), Some("1"));
        std::env::remove_var("MUSICWASP_GATE_PROBE");
    }
}
