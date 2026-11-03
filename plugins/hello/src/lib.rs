//! Example YACE plugin (spec §18.16): shows the whole SDK surface in a few
//! lines — commands, settings, a status item, events, reading and editing
//! the active document, and a background job.

use serde_json::Value;
use yace_plugin::{Edit, Error, Host, Job, Level, Plugin, Result, StatusItem, export_plugin};

pub struct Hello {
    host: Host,
    saves: u32,
}

impl Plugin for Hello {
    fn new(host: Host) -> Self {
        Hello { host, saves: 0 }
    }

    fn activate(&mut self) -> Result<()> {
        self.host.log(Level::Info, "Hello activated");
        self.show_saves()
    }

    fn command(&mut self, id: &str, _args: Option<Value>) -> Result<()> {
        match id {
            "hello.greet" => {
                let greeting: String = self.host.setting("greeting")?;
                let exclaim: bool = self.host.setting("exclaim")?;
                let roots = self.host.roots()?.len();
                self.host.info(&format!(
                    "{greeting}{} ({roots} workspace folder{})",
                    if exclaim { "!" } else { "" },
                    if roots == 1 { "" } else { "s" }
                ));
                Ok(())
            }
            "hello.wordCount" => {
                let doc = self.host.active_document()?.ok_or_else(|| Error::new("no active editor"))?;
                let text = self.host.document_text(doc.doc, 0, u64::MAX)?;
                self.host.info(&format!("{} words", text.split_whitespace().count()));
                Ok(())
            }
            "hello.insertDate" => {
                let doc = self.host.active_document()?.ok_or_else(|| Error::new("no active editor"))?;
                let at = doc.selections.first().map(|s| s.head).unwrap_or(0);
                let days = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() / 86_400)
                    .unwrap_or(0);
                let text = civil_date(days as i64);
                self.host.apply_edits(doc.doc, doc.version, &[Edit { start: at, end: at, text }])
            }
            "hello.slowCount" => {
                self.host.spawn_job(|job: Job| {
                    let mut n = 0u64;
                    for i in 0..1_000_000u64 {
                        if i % 10_000 == 0 && job.cancelled() {
                            return;
                        }
                        n += i % 2;
                    }
                    job.post(&format!("counted {n} odd numbers"));
                })?;
                Ok(())
            }
            _ => Err(Error::unsupported()),
        }
    }

    fn event(&mut self, event: Value) {
        if event["type"] == "documentSaved" {
            self.saves += 1;
            let _ = self.show_saves();
        }
    }

    fn message(&mut self, data: &str) {
        self.host.info(data);
    }
}

impl Hello {
    fn show_saves(&self) -> Result<()> {
        let item = StatusItem {
            text: format!("💾 {}", self.saves),
            tooltip: "Documents saved this session (Hello plugin)".into(),
            command: "hello.greet".into(),
            ..Default::default()
        };
        self.host.set_status_item("hello.saves", Some(&item))
    }
}

/// Days since 1970-01-01 → `YYYY-MM-DD` (Howard Hinnant's algorithm).
fn civil_date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

export_plugin!(Hello);

#[cfg(test)]
mod tests {
    use serde_json::json;
    use yace_plugin::testing::MockHost;

    use super::*;

    #[test]
    fn greets_counts_and_edits() {
        let mock = MockHost::new();
        mock.with(|s| {
            s.settings.insert("greeting".into(), json!("Hi"));
            s.settings.insert("exclaim".into(), json!(false));
        });
        let mut p = Hello::new(mock.host());
        p.activate().unwrap();
        p.command("hello.greet", None).unwrap();
        assert_eq!(mock.with(|s| s.notifications.last().unwrap().1.clone()), "Hi (0 workspace folders)");
        p.event(json!({"type": "documentSaved", "doc": 1}));
        assert_eq!(mock.with(|s| s.status_items["hello.saves"].clone().unwrap().text), "💾 1");
        p.command("hello.slowCount", None).unwrap();
        assert_eq!(mock.with(|s| s.messages.clone()), vec!["counted 500000 odd numbers".to_string()]);
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(20_395), "2025-11-03");
    }
}
