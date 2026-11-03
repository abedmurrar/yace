//! A mock editor for unit-testing plugins without YACE (spec §18.15).
//!
//! ```ignore
//! let mock = MockHost::new();
//! mock.with(|s| s.settings.insert("greeting".into(), json!("Hi")));
//! let mut p = MyPlugin::new(mock.host());
//! p.command("hello.greet", None).unwrap();
//! assert_eq!(mock.with(|s| s.notifications.clone()), vec![(Level::Info, "Hi".into())]);
//! ```
//!
//! State is per thread (each test has its own). Jobs run immediately on the
//! calling thread; messages they post are collected in `messages`.

use std::{cell::RefCell, collections::BTreeMap, ffi::c_void};

use serde_json::Value;
use yace_plugin_api::{
    ABI_VERSION, YACE_E_FAILED, YACE_E_INVALID, YACE_E_NOT_FOUND, YACE_E_STALE, YACE_OK, YaceBuf, YaceHandle, YaceHost,
    YaceHostCtx, YaceJobFn, YaceStatus, YaceStr,
};

use crate::{ActiveDocument, Decoration, Edit, Host, Level, QuickOpenItem, Root, StatusItem, TerminalSpec, ViewItem};

/// Everything the mock editor knows and every call it received.
#[derive(Debug, Default)]
pub struct MockState {
    pub settings: BTreeMap<String, Value>,
    pub roots: Vec<Root>,
    pub active: Option<ActiveDocument>,
    /// Document text by handle; `apply_edits` changes it.
    pub documents: BTreeMap<u64, String>,
    /// Document versions; `apply_edits` checks and bumps them.
    pub versions: BTreeMap<u64, u64>,
    pub logs: Vec<(Level, String)>,
    pub notifications: Vec<(Level, String)>,
    pub executed: Vec<(String, Option<Value>)>,
    pub opened: Vec<(String, u32, u32)>,
    pub status_items: BTreeMap<String, Option<StatusItem>>,
    pub views: BTreeMap<String, Vec<ViewItem>>,
    pub decorations: BTreeMap<u64, Vec<Decoration>>,
    pub quick_open: Vec<(u64, Vec<QuickOpenItem>)>,
    pub terminals: Vec<TerminalSpec>,
    pub messages: Vec<String>,
    pub jobs: u64,
    last_error: String,
}

thread_local! {
    static STATE: RefCell<MockState> = RefCell::new(MockState::default());
}

fn with<R>(f: impl FnOnce(&mut MockState) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

fn text(v: YaceStr) -> String {
    if v.ptr.is_null() {
        return String::new();
    }
    // SAFETY: the SDK passes valid borrowed strings.
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    String::from_utf8_lossy(bytes).into_owned()
}

fn fail(status: YaceStatus, msg: impl Into<String>) -> YaceStatus {
    with(|s| s.last_error = msg.into());
    status
}

fn fill(out: *mut YaceBuf, data: String) -> YaceStatus {
    if out.is_null() {
        return YACE_E_INVALID;
    }
    let mut bytes = data.into_bytes().into_boxed_slice();
    let (ptr, len) = (bytes.as_mut_ptr(), bytes.len());
    std::mem::forget(bytes);
    // SAFETY: checked non-null.
    unsafe { *out = YaceBuf { ptr, len, cap: len } };
    YACE_OK
}

fn level(l: u32) -> Level {
    match l {
        0 => Level::Debug,
        1 => Level::Info,
        2 => Level::Warning,
        _ => Level::Error,
    }
}

extern "C" fn log(_: *mut YaceHostCtx, l: u32, m: YaceStr) {
    with(|s| s.logs.push((level(l), text(m))));
}
extern "C" fn notify(_: *mut YaceHostCtx, l: u32, m: YaceStr) {
    with(|s| s.notifications.push((level(l), text(m))));
}
extern "C" fn last_error(_: *mut YaceHostCtx) -> YaceStr {
    // Leaked per call: fine for tests.
    let e: &'static str = Box::leak(with(|s| s.last_error.clone()).into_boxed_str());
    YaceStr::from_str(e)
}
extern "C" fn free(_: *mut YaceHostCtx, buf: YaceBuf) {
    if !buf.ptr.is_null() {
        // SAFETY: allocated by `fill` as a boxed slice of `cap` bytes.
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(buf.ptr, buf.cap)) });
    }
}
extern "C" fn get_setting(_: *mut YaceHostCtx, key: YaceStr, out: *mut YaceBuf) -> YaceStatus {
    let key = text(key);
    match with(|s| s.settings.get(&key).cloned()) {
        Some(v) => fill(out, v.to_string()),
        None => fail(YACE_E_NOT_FOUND, format!("no setting `{key}`")),
    }
}
extern "C" fn workspace_roots(_: *mut YaceHostCtx, out: *mut YaceBuf) -> YaceStatus {
    fill(out, with(|s| serde_json::to_string(&s.roots).unwrap_or_default()))
}
extern "C" fn active_document(_: *mut YaceHostCtx, out: *mut YaceBuf) -> YaceStatus {
    fill(out, with(|s| serde_json::to_string(&s.active).unwrap_or_default()))
}
extern "C" fn document_text(
    _: *mut YaceHostCtx,
    doc: YaceHandle,
    start: u64,
    end: u64,
    out: *mut YaceBuf,
) -> YaceStatus {
    let Some(t) = with(|s| s.documents.get(&doc).cloned()) else { return fail(YACE_E_NOT_FOUND, "no such document") };
    let chars: Vec<char> = t.chars().collect();
    let (a, b) = (start.min(chars.len() as u64) as usize, end.min(chars.len() as u64) as usize);
    fill(out, chars[a..b.max(a)].iter().collect())
}
extern "C" fn apply_edits(_: *mut YaceHostCtx, doc: YaceHandle, version: u64, edits: YaceStr) -> YaceStatus {
    let Ok(mut edits) = serde_json::from_str::<Vec<Edit>>(&text(edits)) else {
        return fail(YACE_E_INVALID, "bad edits");
    };
    with(|s| {
        if s.versions.get(&doc).copied().unwrap_or(0) != version {
            s.last_error = "stale".into();
            return YACE_E_STALE;
        }
        let Some(t) = s.documents.get_mut(&doc) else { return YACE_E_NOT_FOUND };
        let mut chars: Vec<char> = t.chars().collect();
        edits.sort_by_key(|e| std::cmp::Reverse(e.start));
        for e in &edits {
            let (a, b) = (e.start as usize, (e.end as usize).min(chars.len()));
            chars.splice(a..b.max(a), e.text.chars());
        }
        *t = chars.into_iter().collect();
        *s.versions.entry(doc).or_default() += 1;
        YACE_OK
    })
}
extern "C" fn open_file(_: *mut YaceHostCtx, path: YaceStr, line: u32, col: u32) -> YaceStatus {
    with(|s| s.opened.push((text(path), line, col)));
    YACE_OK
}
extern "C" fn execute_command(_: *mut YaceHostCtx, id: YaceStr, args: YaceStr) -> YaceStatus {
    let args = text(args);
    with(|s| s.executed.push((text(id), serde_json::from_str(&args).ok())));
    YACE_OK
}
extern "C" fn set_status_item(_: *mut YaceHostCtx, id: YaceStr, item: YaceStr) -> YaceStatus {
    let item: Option<StatusItem> = match serde_json::from_str(&text(item)) {
        Ok(i) => i,
        Err(e) => return fail(YACE_E_INVALID, e.to_string()),
    };
    with(|s| s.status_items.insert(text(id), item));
    YACE_OK
}
extern "C" fn set_view_items(_: *mut YaceHostCtx, view: YaceStr, items: YaceStr) -> YaceStatus {
    match serde_json::from_str::<Vec<ViewItem>>(&text(items)) {
        Ok(items) => {
            with(|s| s.views.insert(text(view), items));
            YACE_OK
        }
        Err(e) => fail(YACE_E_INVALID, e.to_string()),
    }
}
extern "C" fn set_decorations(_: *mut YaceHostCtx, root: YaceHandle, items: YaceStr) -> YaceStatus {
    match serde_json::from_str::<Vec<Decoration>>(&text(items)) {
        Ok(items) => {
            with(|s| s.decorations.insert(root, items));
            YACE_OK
        }
        Err(e) => fail(YACE_E_INVALID, e.to_string()),
    }
}
extern "C" fn quick_open_results(_: *mut YaceHostCtx, request: u64, items: YaceStr) -> YaceStatus {
    match serde_json::from_str::<Vec<QuickOpenItem>>(&text(items)) {
        Ok(items) => {
            with(|s| s.quick_open.push((request, items)));
            YACE_OK
        }
        Err(e) => fail(YACE_E_INVALID, e.to_string()),
    }
}
extern "C" fn create_terminal(_: *mut YaceHostCtx, spec: YaceStr, out: *mut YaceHandle) -> YaceStatus {
    match serde_json::from_str::<TerminalSpec>(&text(spec)) {
        Ok(spec) => {
            let id = with(|s| {
                s.terminals.push(spec);
                s.terminals.len() as u64
            });
            if !out.is_null() {
                // SAFETY: checked non-null.
                unsafe { *out = id };
            }
            YACE_OK
        }
        Err(e) => fail(YACE_E_FAILED, e.to_string()),
    }
}
extern "C" fn spawn_job(_: *mut YaceHostCtx, job: YaceJobFn, data: *mut c_void, out: *mut YaceHandle) -> YaceStatus {
    let id = with(|s| {
        s.jobs += 1;
        s.jobs
    });
    if !out.is_null() {
        // SAFETY: checked non-null.
        unsafe { *out = id };
    }
    job(data, id);
    YACE_OK
}
extern "C" fn job_cancelled(_: *mut YaceHostCtx, _job: YaceHandle) -> bool {
    false
}
extern "C" fn post_message(_: *mut YaceHostCtx, data: YaceStr) -> YaceStatus {
    with(|s| s.messages.push(text(data)));
    YACE_OK
}

/// A fake editor host for tests. Its state lives in a thread-local.
pub struct MockHost {
    table: Box<YaceHost>,
}

impl Default for MockHost {
    fn default() -> Self {
        Self::new()
    }
}

impl MockHost {
    /// A fresh mock (resets this thread's state).
    pub fn new() -> Self {
        with(|s| *s = MockState::default());
        let table = Box::new(YaceHost {
            size: std::mem::size_of::<YaceHost>() as u32,
            abi_version: ABI_VERSION,
            ctx: std::ptr::null_mut(),
            log,
            notify,
            last_error,
            free,
            get_setting,
            workspace_roots,
            active_document,
            document_text,
            apply_edits,
            open_file,
            execute_command,
            set_status_item,
            set_view_items,
            set_decorations,
            quick_open_results,
            create_terminal,
            spawn_job,
            job_cancelled,
            post_message,
        });
        Self { table }
    }

    pub fn host(&self) -> Host {
        // SAFETY: the table lives as long as `self`.
        unsafe { Host::from_raw(&*self.table) }
    }

    /// The raw table (to test `export_plugin!` entry points).
    pub fn raw(&self) -> *const YaceHost {
        &*self.table
    }

    /// Read or change the mock's state.
    pub fn with<R>(&self, f: impl FnOnce(&mut MockState) -> R) -> R {
        with(f)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{Error, Job, Plugin, Result};

    struct Counter {
        host: Host,
        n: u32,
    }

    impl Plugin for Counter {
        fn new(host: Host) -> Self {
            Counter { host, n: 0 }
        }
        fn command(&mut self, id: &str, args: Option<Value>) -> Result<()> {
            match id {
                "c.inc" => {
                    self.n += args.and_then(|a| a.as_u64()).unwrap_or(1) as u32;
                    let greeting: String = self.host.setting("greeting")?;
                    self.host.info(&format!("{greeting} {}", self.n));
                    self.host.set_status_item(
                        "c.status",
                        Some(&StatusItem { text: self.n.to_string(), ..Default::default() }),
                    )
                }
                "c.edit" => {
                    let d = self.host.active_document()?.ok_or_else(|| Error::new("no document"))?;
                    let t = self.host.document_text(d.doc, 0, u64::MAX)?;
                    self.host.apply_edits(
                        d.doc,
                        d.version,
                        &[Edit { start: 0, end: 0, text: format!("{} chars: ", t.chars().count()) }],
                    )
                }
                "c.job" => {
                    self.host.spawn_job(|job: Job| {
                        job.post(&format!("done {}", job.id));
                    })?;
                    Ok(())
                }
                "c.panic" => panic!("oh no"),
                _ => Err(Error::unsupported()),
            }
        }
        fn message(&mut self, data: &str) {
            self.host.log(Level::Info, data);
        }
    }

    crate::export_plugin!(Counter);

    #[test]
    fn plugins_can_be_tested_with_the_mock() {
        let mock = MockHost::new();
        mock.with(|s| {
            s.settings.insert("greeting".into(), json!("Count"));
            s.documents.insert(7, "héllo".into());
            s.versions.insert(7, 3);
            s.active =
                Some(ActiveDocument { doc: 7, version: 3, path: None, language: "plain".into(), selections: vec![] });
        });
        let mut p = Counter::new(mock.host());
        p.command("c.inc", Some(json!(2))).unwrap();
        mock.with(|s| {
            assert_eq!(s.notifications, vec![(Level::Info, "Count 2".to_string())]);
            assert_eq!(s.status_items["c.status"].as_ref().unwrap().text, "2");
        });
        p.command("c.edit", None).unwrap();
        mock.with(|s| assert_eq!(s.documents[&7], "5 chars: héllo"));
        // A stale version is refused.
        mock.with(|s| s.active.as_mut().unwrap().version = 3);
        let e = p.command("c.edit", None).unwrap_err();
        assert_eq!(e.status, YACE_E_STALE);
        p.command("c.job", None).unwrap();
        mock.with(|s| assert_eq!(s.messages, vec!["done 1".to_string()]));
        // Missing setting → error with the host's message.
        mock.with(|s| s.settings.clear());
        let e = p.command("c.inc", None).unwrap_err();
        assert!(e.message.contains("no setting `greeting`"), "{e}");
    }

    #[test]
    fn the_exported_entry_point_validates_and_catches_panics() {
        let mock = MockHost::new();
        let mut table = yace_plugin_api::YacePlugin {
            size: std::mem::size_of::<yace_plugin_api::YacePlugin>() as u32,
            abi_version: 0,
            instance: std::ptr::null_mut(),
            activate: None,
            deactivate: None,
            on_command: None,
            on_event: None,
            on_message: None,
            on_view_action: None,
            on_quick_open: None,
            destroy: None,
        };
        // SAFETY: valid pointers.
        assert_eq!(unsafe { yace_plugin_init(2, mock.raw(), &mut table) }, yace_plugin_api::YACE_E_ABI);
        assert_eq!(table.abi_version, 0, "nothing written for the wrong ABI");
        // SAFETY: valid pointers.
        assert_eq!(unsafe { yace_plugin_init(ABI_VERSION, std::ptr::null(), &mut table) }, YACE_E_INVALID);
        // SAFETY: valid pointers.
        assert_eq!(unsafe { yace_plugin_init(ABI_VERSION, mock.raw(), &mut table) }, YACE_OK);
        assert_eq!(table.abi_version, ABI_VERSION);
        let cmd = table.on_command.unwrap();
        let st = cmd(table.instance, YaceStr::from_str("c.panic"), YaceStr::EMPTY);
        assert_eq!(st, yace_plugin_api::YACE_E_PANIC);
        assert!(mock.with(|s| s.logs.iter().any(|(_, m)| m.contains("panicked: oh no"))));
        let st = cmd(table.instance, YaceStr::from_str("nope"), YaceStr::EMPTY);
        assert_eq!(st, yace_plugin_api::YACE_E_UNSUPPORTED);
        // A smaller table (an older editor): only that many bytes written.
        #[repr(C, align(8))]
        struct Aligned([u8; 128]);
        let mut small = Aligned([0u8; 128]);
        let sp = small.0.as_mut_ptr() as *mut yace_plugin_api::YacePlugin;
        // SAFETY: `size` is the first field; we only read the header back.
        unsafe {
            (*sp).size = 24;
            assert_eq!(yace_plugin_init(ABI_VERSION, mock.raw(), sp), YACE_OK);
            assert_eq!((*sp).size, 24);
        }
        assert!(small.0[24..].iter().all(|b| *b == 0), "nothing written past the given size");
        (table.destroy.unwrap())(table.instance);
    }
}
