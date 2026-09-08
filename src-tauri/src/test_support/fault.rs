//! Fault injection (Reliability Harness Phase 1).
//!
//! Deterministic fault points for scenario and recovery tests:
//!
//! ```text
//! JR_FAULT_INJECT="updater.download.mid=error:boom,app.start.after_runtime=panic"
//! ```
//!
//! - Keys are dotted fault-point names (see updater-trust task for the full
//!   updater matrix; the grammar lives here).
//! - Actions: `error:<msg>`, `panic`, `delay:<ms>`.
//! - `panic` and `delay` compile only in debug builds; in release builds a
//!   triggered point is a no-op returning `Ok(())` — zero behavior change.
//!   Env parsing therefore also only matters in tests/debug scenarios.

use std::collections::HashMap;

/// The parsed, active fault rules for one process.
#[derive(Debug, Default, Clone)]
pub struct FaultInjector {
    rules: HashMap<String, FaultAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FaultAction {
    Error(String),
    Panic,
    DelayMs(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaultError {
    key: String,
    message: String,
}

impl FaultError {
    pub fn key(&self) -> &str {
        &self.key
    }
}

impl std::error::Error for FaultError {}
impl std::fmt::Display for FaultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "injected fault at {}: {}", self.key, self.message)
    }
}

impl FaultInjector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load rules from a `JR_FAULT_INJECT`-style string
    /// (`key1=error:msg,key2=panic,key3=delay:1200`). Unknown actions are
    /// ignored (forward-compatible with future action kinds).
    pub fn parse(spec: &str) -> Self {
        let mut injector = Self::new();
        for rule in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let Some((key, action)) = rule.split_once('=') else {
                continue;
            };
            let key = key.trim().to_string();
            let action_text = action.trim();
            let action = match action_text.split_once(':') {
                Some(("error", msg)) => FaultAction::Error(msg.trim().to_string()),
                Some(("delay", ms)) => FaultAction::DelayMs(ms.trim().parse().unwrap_or(0)),
                _ if action_text == "panic" => FaultAction::Panic,
                _ => continue,
            };
            injector.rules.insert(key, action);
        }
        injector
    }

    /// Register a fault for `key` at runtime (test scenario builders).
    pub fn arm(&mut self, key: &str, action: FaultAction) {
        self.rules.insert(key.to_string(), action);
    }

    pub fn has(&self, key: &str) -> bool {
        self.rules.contains_key(key)
    }

    /// Fire the fault point `key`, if armed. Returns `Ok(())` when the point
    /// is unarmed, or when `delay`/`panic` are neutralized (release builds).
    pub fn trigger(&self, key: &str) -> Result<(), FaultError> {
        let Some(action) = self.rules.get(key) else {
            return Ok(());
        };
        match action {
            FaultAction::Error(msg) => Err(FaultError {
                key: key.to_string(),
                message: msg.clone(),
            }),
            #[cfg(debug_assertions)]
            FaultAction::Panic => panic!("injected panic at fault point {key}"),
            #[cfg(not(debug_assertions))]
            FaultAction::Panic => Ok(()),
            #[cfg(debug_assertions)]
            FaultAction::DelayMs(ms) => {
                std::thread::sleep(std::time::Duration::from_millis(*ms));
                Ok(())
            }
            #[cfg(not(debug_assertions))]
            FaultAction::DelayMs(_) => Ok(()),
        }
    }
}

/// Default injector from the environment (`JR_FAULT_INJECT`). Call once per
/// test binary like any other env-driven config.
pub fn from_env() -> FaultInjector {
    match std::env::var("JR_FAULT_INJECT") {
        Ok(spec) => FaultInjector::parse(&spec),
        Err(_) => FaultInjector::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_spec_has_no_rules() {
        let injector = FaultInjector::parse("");
        assert!(!injector.has("anything"));
        assert!(injector.trigger("anything").is_ok());
    }

    #[test]
    fn parses_error_delay_and_panic_actions() {
        let injector = FaultInjector::parse(
            "updater.download.mid=error:boom,app.start.after_runtime=panic,slow.point=delay:10",
        );
        assert!(injector.has("updater.download.mid"));
        assert!(injector.has("app.start.after_runtime"));
        assert!(injector.has("slow.point"));
        assert!(injector.trigger("updater.download.mid").is_err());
    }

    #[test]
    fn error_action_reports_key_and_message() {
        let injector = FaultInjector::parse("updater.swap.before=error:disk full");
        let err = injector.trigger("updater.swap.before").unwrap_err();
        assert_eq!(
            err.to_string(),
            "injected fault at updater.swap.before: disk full"
        );
        assert_eq!(err.key(), "updater.swap.before");
    }

    #[test]
    fn unarmed_point_stays_ok() {
        let injector = FaultInjector::parse("a=error:x");
        assert!(injector.trigger("b").is_ok());
    }

    #[test]
    fn malformed_rules_are_ignored() {
        let injector = FaultInjector::parse("noequals,numeric=delay:abc,unknown=explode:now");
        assert!(!injector.has("noequals"));
        assert!(!injector.has("unknown"));
        assert!(injector.has("numeric"));
        assert!(injector.trigger("numeric").is_ok()); // delay:0 parses, no-op
        assert!(injector.trigger("noequals").is_ok());
    }

    #[test]
    fn arm_adds_runtime_rules() {
        let mut injector = FaultInjector::new();
        injector.arm("x", FaultAction::Error("late".into()));
        assert!(injector.trigger("x").is_err());
    }

    #[test]
    fn error_message_cannot_carry_over_from_another_rule() {
        // The message lives inside the rule value; two rules are isolated.
        let injector = FaultInjector::parse("a=error:one,b=error:two");
        assert_eq!(
            injector.trigger("a").unwrap_err().to_string(),
            "injected fault at a: one"
        );
        assert_eq!(
            injector.trigger("b").unwrap_err().to_string(),
            "injected fault at b: two"
        );
    }
}
