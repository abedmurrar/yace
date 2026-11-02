//! Terminal lifecycle (spec §26.3): enter raw/alternate-screen mode and
//! always restore it — on normal exit, on panic and on signals.

use std::{
    io::{self, Stdout, Write},
    sync::atomic::{AtomicBool, Ordering},
};

use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    crossterm::{
        cursor,
        event::{
            DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste, EnableFocusChange,
            EnableMouseCapture, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
        },
        execute,
        terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
    },
};

static ACTIVE: AtomicBool = AtomicBool::new(false);
static ENHANCED: AtomicBool = AtomicBool::new(false);

pub type Term = Terminal<CrosstermBackend<Stdout>>;

/// Put the terminal into application mode. Returns the terminal and whether
/// the enhanced (Kitty) keyboard protocol is active.
pub fn enter(want_enhanced: bool) -> io::Result<(Term, bool)> {
    terminal::enable_raw_mode()?;
    ACTIVE.store(true, Ordering::SeqCst);
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste, EnableFocusChange)?;
    let enhanced = want_enhanced && terminal::supports_keyboard_enhancement().unwrap_or(false);
    if enhanced {
        // Only disambiguation: plain text keeps arriving as text, while
        // Ctrl/Alt combinations are reported unambiguously.
        execute!(out, PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES))?;
        ENHANCED.store(true, Ordering::SeqCst);
    }
    let term = Terminal::new(CrosstermBackend::new(out))?;
    Ok((term, enhanced))
}

/// Restore the terminal. Safe to call more than once and from a panic hook.
pub fn restore() {
    if !ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    let mut out = io::stdout();
    if ENHANCED.swap(false, Ordering::SeqCst) {
        let _ = execute!(out, PopKeyboardEnhancementFlags);
    }
    let _ = execute!(
        out,
        DisableFocusChange,
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen,
        cursor::SetCursorStyle::DefaultUserShape,
        cursor::Show
    );
    let _ = terminal::disable_raw_mode();
    let _ = out.flush();
}

/// Restore the terminal before the default panic message is printed, so
/// the message is readable and the shell is not left in raw mode.
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        default(info);
    }));
}

/// Restores on drop (covers early returns and `?` in `main`).
pub struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        restore();
    }
}

/// Write raw bytes to the terminal (OSC 52 clipboard, window title).
pub fn write_raw(bytes: &[u8]) {
    let mut out = io::stdout();
    let _ = out.write_all(bytes);
    let _ = out.flush();
}
