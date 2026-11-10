//! Test fixture for key capture (see yace-plugin.toml).

use yace_plugin::{Edit, Host, Key, Plugin, Result, export_plugin};

struct Keys {
    host: Host,
}

impl Keys {
    fn cursor(&self) -> Option<(u64, u64, u64)> {
        let a = self.host.active_document().ok()??;
        let head = a.selections.first().map(|s| s.head)?;
        Some((a.doc, a.version, head))
    }
}

impl Plugin for Keys {
    fn new(host: Host) -> Self {
        Self { host }
    }

    fn command(&mut self, id: &str, _args: Option<serde_json::Value>) -> Result<()> {
        if id == "testkeys.on" {
            self.host.capture_keys(true)?;
            self.host.set_cursor_style("block")?;
        }
        Ok(())
    }

    fn key(&mut self, key: &Key) -> bool {
        let Some((doc, version, head)) = self.cursor() else { return false };
        match key.text.as_deref() {
            Some("x") => {
                let _ =
                    self.host.apply_edits(doc, version, &[Edit { start: head, end: head + 1, text: String::new() }]);
                if let Some((doc, version, _)) = self.cursor() {
                    let _ = self.host.set_selections(doc, version, &[(head, head)]);
                }
                true
            }
            Some("l") => {
                let _ = self.host.set_selections(doc, version, &[(head + 1, head + 1)]);
                true
            }
            Some("i") => {
                let _ = self.host.capture_keys(false);
                let _ = self.host.set_cursor_style("bar");
                true
            }
            _ => false,
        }
    }
}

export_plugin!(Keys);
