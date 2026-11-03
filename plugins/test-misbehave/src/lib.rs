//! Test fixture: misbehaves on purpose through the raw ABI. The mode comes
//! from `YACE_TEST_PLUGIN_MODE` at init: `ok` (default), `abi`, `fail`,
//! `small-table`, `wrong-abi`. In `ok` mode, `misbehave.run` sends the host
//! malformed input and logs each status as `name=status`.

use std::ffi::c_void;

use yace_plugin_api::*;

static mut HOST: *const YaceHost = std::ptr::null();

fn host() -> &'static YaceHost {
    // SAFETY: set once in init; the table outlives the plugin.
    unsafe { &*HOST }
}

fn log(msg: &str) {
    let h = host();
    (h.log)(h.ctx, YACE_LEVEL_INFO, YaceStr::from_str(msg));
}

fn st(v: &str) -> YaceStr {
    YaceStr::from_str(v)
}

extern "C" fn on_command(_inst: *mut c_void, id: YaceStr, _args: YaceStr) -> YaceStatus {
    // SAFETY: borrowed for the call.
    let id = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(id.ptr, id.len)) };
    let h = host();
    match id {
        "misbehave.run" => {
            let mut out = YaceBuf::EMPTY;
            let bad_utf8 = [0xffu8, 0xfe, 0x41];
            let results = [
                ("null-string", (h.set_status_item)(h.ctx, YaceStr { ptr: std::ptr::null(), len: 5 }, st("{}"))),
                ("invalid-utf8", (h.set_status_item)(h.ctx, YaceStr { ptr: bad_utf8.as_ptr(), len: 3 }, st("null"))),
                (
                    "huge-length",
                    (h.set_status_item)(h.ctx, YaceStr { ptr: bad_utf8.as_ptr(), len: usize::MAX }, st("null")),
                ),
                ("null-out", (h.get_setting)(h.ctx, st("x"), std::ptr::null_mut())),
                ("unknown-setting", (h.get_setting)(h.ctx, st("nope"), &mut out)),
                ("unknown-doc", (h.apply_edits)(h.ctx, 999_999, 0, st("[]"))),
                ("no-capability", (h.document_text)(h.ctx, 1, 0, 10, &mut out)),
                ("bad-json", (h.set_status_item)(h.ctx, st("misbehave.status"), st("{not json"))),
                ("undeclared-status", (h.set_status_item)(h.ctx, st("someone.else"), st("{\"text\":\"x\"}"))),
                ("undeclared-view", (h.set_view_items)(h.ctx, st("no.view"), st("[]"))),
                ("no-terminal-capability", (h.create_terminal)(h.ctx, st("{}"), std::ptr::null_mut())),
                ("good-status", (h.set_status_item)(h.ctx, st("misbehave.status"), st("{\"text\":\"ok\"}"))),
            ];
            (h.free)(h.ctx, out);
            for (name, status) in results {
                log(&format!("{name}={status}"));
            }
            // last_error after a failure describes it.
            let mut out = YaceBuf::EMPTY;
            let _ = (h.get_setting)(h.ctx, st("nope"), &mut out);
            (h.free)(h.ctx, out);
            let e = (h.last_error)(h.ctx);
            // SAFETY: valid until the next host call.
            let e = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(e.ptr, e.len)) }.to_string();
            log(&format!("last-error={e}"));
            YACE_OK
        }
        "misbehave.slow" => {
            std::thread::sleep(std::time::Duration::from_millis(150));
            YACE_OK
        }
        "misbehave.fail" => YACE_E_FAILED,
        _ => YACE_E_UNSUPPORTED,
    }
}

/// # Safety
/// Called by the editor with valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yace_plugin_init(abi: u32, h: *const YaceHost, out: *mut YacePlugin) -> YaceStatus {
    let mode = std::env::var("YACE_TEST_PLUGIN_MODE").unwrap_or_default();
    if abi != ABI_VERSION || mode == "abi" {
        return YACE_E_ABI;
    }
    if mode == "fail" {
        return YACE_E_FAILED;
    }
    // SAFETY: single-threaded init.
    unsafe { HOST = h };
    // SAFETY: the editor passes a writable table of `size` bytes.
    unsafe {
        let o = &mut *out;
        o.abi_version = if mode == "wrong-abi" { 99 } else { ABI_VERSION };
        o.on_command = Some(on_command);
        o.size = if mode == "small-table" { 8 } else { std::mem::size_of::<YacePlugin>() as u32 };
    }
    YACE_OK
}
