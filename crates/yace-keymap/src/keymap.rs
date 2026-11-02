//! Layered keymaps and the chord-aware resolver.
//!
//! Precedence (spec §5.4): defaults < plugins < user. Bindings are kept in
//! precedence order; resolution scans from the end so later (higher
//! precedence) bindings win. A user binding whose command starts with `-`
//! removes earlier matching bindings, like VS Code's `keybindings.json`.

use std::time::{Duration, Instant};

use serde::Deserialize;
use smallvec::SmallVec;
use thiserror::Error;

use crate::{
    key::{KeyPress, KeySeq, ParseError},
    when::{Context, When, WhenError},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    Default,
    Plugin,
    User,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub keys: KeySeq,
    pub command: String,
    pub args: Option<serde_json::Value>,
    pub when: When,
    pub source: Source,
}

/// One entry of the user's `keybindings.toml` (`[[bindings]]` tables).
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BindingSpec {
    /// Key sequence, e.g. `"ctrl+k ctrl+t"`. May be omitted for removals.
    #[serde(default)]
    pub key: Option<String>,
    pub command: String,
    #[serde(default)]
    pub when: Option<String>,
    #[serde(default)]
    pub args: Option<serde_json::Value>,
}

#[derive(Debug, Error, PartialEq)]
pub enum BindingError {
    #[error("binding {index} (`{command}`): {source}")]
    Key { index: usize, command: String, source: ParseError },
    #[error("binding {index} (`{command}`): {source}")]
    When { index: usize, command: String, source: WhenError },
    #[error("binding {index} (`{command}`): a key is required")]
    MissingKey { index: usize, command: String },
}

#[derive(Clone, Debug, Default)]
pub struct Keymap {
    bindings: Vec<Binding>,
}

impl Keymap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    pub fn push(&mut self, b: Binding) {
        self.bindings.push(b);
    }

    /// Add a binding from strings; panics on invalid built-in tables (tests
    /// guarantee the defaults parse).
    pub fn add(&mut self, keys: &str, command: &str, when: &str, source: Source) {
        self.bindings.push(Binding {
            keys: keys.parse().unwrap_or_else(|e| panic!("bad built-in key `{keys}`: {e}")),
            command: command.to_string(),
            args: None,
            when: When::parse(when).unwrap_or_else(|e| panic!("bad built-in when `{when}`: {e}")),
            source,
        });
    }

    /// Apply a layer of specs (user or plugin). Valid entries are applied
    /// even when others fail; all errors are returned for reporting.
    pub fn apply_specs(&mut self, specs: &[BindingSpec], source: Source) -> Vec<BindingError> {
        let mut errors = Vec::new();
        for (index, spec) in specs.iter().enumerate() {
            let keys = match spec.key.as_deref().map(str::parse::<KeySeq>) {
                Some(Ok(k)) => Some(k),
                Some(Err(source)) => {
                    errors.push(BindingError::Key { index, command: spec.command.clone(), source });
                    continue;
                }
                None => None,
            };
            let when = match spec.when.as_deref().map(When::parse) {
                Some(Ok(w)) => Some(w),
                Some(Err(source)) => {
                    errors.push(BindingError::When { index, command: spec.command.clone(), source });
                    continue;
                }
                None => None,
            };
            if let Some(cmd) = spec.command.strip_prefix('-') {
                // Removal: drop earlier bindings for `cmd` (matching key/when if given).
                self.bindings.retain(|b| {
                    !(b.command == cmd
                        && keys.as_ref().is_none_or(|k| *k == b.keys)
                        && when.as_ref().is_none_or(|w| *w == b.when))
                });
                continue;
            }
            let Some(keys) = keys else {
                errors.push(BindingError::MissingKey { index, command: spec.command.clone() });
                continue;
            };
            self.bindings.push(Binding {
                keys,
                command: spec.command.clone(),
                args: spec.args.clone(),
                when: when.unwrap_or_default(),
                source,
            });
        }
        errors
    }

    /// Highest-precedence key sequence bound to `command` (for display).
    pub fn keys_for(&self, command: &str, ctx: Option<&dyn Context>) -> Option<&KeySeq> {
        self.bindings.iter().rev().find(|b| b.command == command && ctx.is_none_or(|c| b.when.eval(c))).map(|b| &b.keys)
    }

    /// Bindings sharing an identical key sequence and when-clause but
    /// targeting different commands; only the last one is reachable.
    pub fn conflicts(&self) -> Vec<Conflict> {
        let mut out: Vec<Conflict> = Vec::new();
        for (i, a) in self.bindings.iter().enumerate() {
            for b in &self.bindings[i + 1..] {
                if a.keys == b.keys && a.when == b.when && a.command != b.command {
                    match out.iter_mut().find(|c| c.keys == a.keys && c.when == a.when) {
                        Some(c) => {
                            if !c.commands.contains(&b.command) {
                                c.commands.push(b.command.clone());
                            }
                        }
                        None => out.push(Conflict {
                            keys: a.keys.clone(),
                            when: a.when.clone(),
                            commands: vec![a.command.clone(), b.command.clone()],
                        }),
                    }
                }
            }
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Conflict {
    pub keys: KeySeq,
    pub when: When,
    /// Commands in precedence order; the last one wins.
    pub commands: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Resolution {
    /// Run this command.
    Command { command: String, args: Option<serde_json::Value> },
    /// A chord prefix was pressed; waiting for the next key.
    Pending(KeySeq),
    /// The key is not bound in this context; the focused component may
    /// handle it (e.g. type it as text).
    Unbound,
    /// A chord was started but the follow-up key matches nothing; the key
    /// is consumed.
    InvalidChord(KeySeq),
}

/// Chord state machine.
#[derive(Debug)]
pub struct Resolver {
    pending: SmallVec<[KeyPress; 2]>,
    started: Option<Instant>,
    pub timeout: Duration,
}

impl Default for Resolver {
    fn default() -> Self {
        Self { pending: SmallVec::new(), started: None, timeout: Duration::from_millis(3000) }
    }
}

impl Resolver {
    pub fn is_pending(&self) -> bool {
        !self.pending.is_empty()
    }
    pub fn pending(&self) -> KeySeq {
        KeySeq(self.pending.clone())
    }
    /// When the pending chord expires (for the event loop's timeout).
    pub fn deadline(&self) -> Option<Instant> {
        self.started.map(|s| s + self.timeout)
    }
    pub fn cancel(&mut self) {
        self.pending.clear();
        self.started = None;
    }
    /// Cancel if the chord timed out; returns true if it did.
    pub fn expire(&mut self, now: Instant) -> bool {
        if self.deadline().is_some_and(|d| now >= d) {
            self.cancel();
            return true;
        }
        false
    }

    pub fn feed(&mut self, keymap: &Keymap, key: KeyPress, ctx: &dyn Context) -> Resolution {
        let mut seq: SmallVec<[KeyPress; 2]> = self.pending.clone();
        seq.push(key);
        let was_pending = !self.pending.is_empty();

        let mut exact: Option<&Binding> = None;
        let mut prefix = false;
        for b in keymap.bindings.iter().rev() {
            if !b.keys.starts_with(&seq) || !b.when.eval(ctx) {
                continue;
            }
            if b.keys.len() == seq.len() {
                if exact.is_none() {
                    exact = Some(b);
                }
            } else {
                prefix = true;
            }
        }
        if prefix {
            // Chords take priority over a shorter exact match (VS Code).
            if self.pending.is_empty() {
                self.started = Some(Instant::now());
            }
            self.pending = seq.clone();
            return Resolution::Pending(KeySeq(seq));
        }
        self.cancel();
        if let Some(b) = exact {
            return Resolution::Command { command: b.command.clone(), args: b.args.clone() };
        }
        if was_pending { Resolution::InvalidChord(KeySeq(seq)) } else { Resolution::Unbound }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        key::{KeyCode, Mods},
        when::ContextSet,
    };

    fn kp(s: &str) -> KeyPress {
        s.parse().unwrap()
    }
    fn cmd(r: Resolution) -> String {
        match r {
            Resolution::Command { command, .. } => command,
            other => panic!("expected command, got {other:?}"),
        }
    }

    fn sample() -> Keymap {
        let mut km = Keymap::new();
        km.add("ctrl+s", "save", "", Source::Default);
        km.add("ctrl+k ctrl+t", "theme", "", Source::Default);
        km.add("ctrl+k ctrl+s", "keys", "", Source::Default);
        km.add("ctrl+c", "copy", "editorFocus", Source::Default);
        km.add("ctrl+c", "interrupt", "terminalFocus", Source::Default);
        km
    }

    #[test]
    fn resolves_by_context() {
        let km = sample();
        let mut r = Resolver::default();
        let ed = ContextSet::new().with("editorFocus");
        let term = ContextSet::new().with("terminalFocus");
        assert_eq!(cmd(r.feed(&km, kp("ctrl+c"), &ed)), "copy");
        assert_eq!(cmd(r.feed(&km, kp("ctrl+c"), &term)), "interrupt");
        assert_eq!(r.feed(&km, kp("ctrl+c"), &ContextSet::new()), Resolution::Unbound);
    }

    #[test]
    fn chords_pending_complete_invalid_and_timeout() {
        let km = sample();
        let mut r = Resolver::default();
        let ctx = ContextSet::new();
        assert!(matches!(r.feed(&km, kp("ctrl+k"), &ctx), Resolution::Pending(_)));
        assert!(r.is_pending());
        assert_eq!(cmd(r.feed(&km, kp("ctrl+t"), &ctx)), "theme");
        assert!(!r.is_pending());
        r.feed(&km, kp("ctrl+k"), &ctx);
        assert!(matches!(r.feed(&km, kp("x"), &ctx), Resolution::InvalidChord(_)));
        r.feed(&km, kp("ctrl+k"), &ctx);
        r.timeout = Duration::from_millis(0);
        assert!(r.expire(Instant::now()));
        assert_eq!(cmd(r.feed(&km, kp("ctrl+s"), &ctx)), "save");
    }

    #[test]
    fn user_layer_overrides_removes_and_reports() {
        let mut km = sample();
        let specs: Vec<BindingSpec> = toml::from_str::<std::collections::HashMap<String, Vec<BindingSpec>>>(
            r#"
            [[bindings]]
            key = "ctrl+s"
            command = "custom.save"
            [[bindings]]
            command = "-theme"
            [[bindings]]
            key = "ctrl+nope"
            command = "broken"
            [[bindings]]
            key = "f5"
            command = "bad.when"
            when = "a &&"
            "#,
        )
        .unwrap()
        .remove("bindings")
        .unwrap();
        let errors = km.apply_specs(&specs, Source::User);
        assert_eq!(errors.len(), 2);
        let mut r = Resolver::default();
        let ctx = ContextSet::new();
        assert_eq!(cmd(r.feed(&km, kp("ctrl+s"), &ctx)), "custom.save");
        r.feed(&km, kp("ctrl+k"), &ctx);
        assert!(matches!(r.feed(&km, kp("ctrl+t"), &ctx), Resolution::InvalidChord(_)), "removed");
        let conflicts = km.conflicts();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].commands, vec!["save", "custom.save"]);
        assert_eq!(km.keys_for("custom.save", None).unwrap().to_string(), "Ctrl+S");
        let _ = KeyPress::new(KeyCode::F(5), Mods::NONE);
    }
}
