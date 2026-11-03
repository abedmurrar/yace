//! The default keymap: VS Code's Linux/Windows defaults (spec §5.1), plus
//! fallbacks for terminals without the enhanced (Kitty) keyboard protocol.
//!
//! Each row is `(keys, command, when)`. `docs/keybindings.md` documents the
//! terminal requirements for every combination.

use crate::keymap::{Keymap, Source};

/// Context keys understood by YACE's built-in bindings.
pub mod ctx {
    pub const EDITOR_FOCUS: &str = "editorFocus";
    pub const EXPLORER_FOCUS: &str = "explorerFocus";
    pub const EDITOR_HAS_SELECTION: &str = "editorHasSelection";
    pub const EDITOR_HAS_MULTIPLE_SELECTIONS: &str = "editorHasMultipleSelections";
    pub const EXPLORER_FILTER_ACTIVE: &str = "explorerFilterActive";
    pub const KEYBOARD_ENHANCED: &str = "keyboardEnhanced";
    pub const MULTIPLE_EDITOR_GROUPS: &str = "multipleEditorGroups";
    pub const NOTIFICATION_VISIBLE: &str = "notificationVisible";
    pub const FIND_WIDGET_VISIBLE: &str = "findWidgetVisible";
    pub const FIND_WIDGET_FOCUS: &str = "findWidgetFocus";
    pub const SEARCH_VIEW_FOCUS: &str = "searchViewFocus";
    pub const SEARCH_INPUT_FOCUS: &str = "searchInputFocus";
    pub const TERMINAL_FOCUS: &str = "terminalFocus";
    pub const PANEL_VISIBLE: &str = "panelVisible";
    pub const SUGGEST_WIDGET_VISIBLE: &str = "suggestWidgetVisible";
    pub const PARAMETER_HINTS_VISIBLE: &str = "parameterHintsVisible";
    pub const EDITOR_HOVER_VISIBLE: &str = "editorHoverVisible";
}

const ED: &str = "editorFocus";

pub const DEFAULTS: &[(&str, &str, &str)] = &[
    // --- Workbench / global -------------------------------------------------
    ("ctrl+shift+p", "workbench.action.showCommands", ""),
    ("f1", "workbench.action.showCommands", ""),
    ("ctrl+p", "workbench.action.quickOpen", ""),
    ("ctrl+e", "workbench.action.quickOpen", ""),
    ("ctrl+alt+-", "workbench.action.navigateBack", ""),
    ("ctrl+shift+-", "workbench.action.navigateForward", ""),
    ("alt+left", "workbench.action.navigateBack", ""),
    ("alt+right", "workbench.action.navigateForward", ""),
    ("ctrl+shift+h", "workbench.action.replaceInFiles", ""),
    ("ctrl+shift+e", "workbench.view.explorer", ""),
    ("ctrl+b", "workbench.action.toggleSidebarVisibility", ""),
    ("ctrl+j", "workbench.action.togglePanel", ""),
    ("ctrl+`", "workbench.action.terminal.toggleTerminal", ""),
    ("ctrl+shift+`", "workbench.action.terminal.new", ""),
    ("ctrl+shift+f", "workbench.action.findInFiles", ""),
    ("ctrl+g", "workbench.action.gotoLine", ""),
    ("ctrl+q", "workbench.action.quit", ""),
    ("ctrl+r", "workbench.action.openRecent", ""),
    ("ctrl+k ctrl+o", "workbench.action.files.openFolder", ""),
    ("ctrl+k f", "workbench.action.closeFolder", ""),
    ("ctrl+k ctrl+t", "workbench.action.selectTheme", ""),
    ("ctrl+k ctrl+s", "workbench.action.openGlobalKeybindingsFile", ""),
    ("ctrl+,", "workbench.action.openSettingsJson", ""),
    ("ctrl+n", "workbench.action.files.newUntitledFile", ""),
    ("ctrl+s", "workbench.action.files.save", ""),
    ("ctrl+shift+s", "workbench.action.files.saveAs", ""),
    ("ctrl+k s", "workbench.action.files.saveAll", ""),
    ("ctrl+w", "workbench.action.closeActiveEditor", ""),
    ("ctrl+f4", "workbench.action.closeActiveEditor", ""),
    ("ctrl+k ctrl+w", "workbench.action.closeAllEditors", ""),
    ("ctrl+k u", "workbench.action.closeUnmodifiedEditors", ""),
    ("ctrl+k w", "workbench.action.closeEditorsInGroup", ""),
    ("ctrl+shift+t", "workbench.action.reopenClosedEditor", ""),
    ("ctrl+pagedown", "workbench.action.nextEditor", ""),
    ("ctrl+pageup", "workbench.action.previousEditor", ""),
    ("ctrl+tab", "workbench.action.openNextRecentlyUsedEditorInGroup", ""),
    ("ctrl+shift+tab", "workbench.action.openPreviousRecentlyUsedEditorInGroup", ""),
    ("ctrl+shift+pageup", "workbench.action.moveEditorLeftInGroup", ""),
    ("ctrl+shift+pagedown", "workbench.action.moveEditorRightInGroup", ""),
    ("ctrl+k enter", "workbench.action.keepEditor", ""),
    ("alt+1", "workbench.action.openEditorAtIndex1", ""),
    ("alt+2", "workbench.action.openEditorAtIndex2", ""),
    ("alt+3", "workbench.action.openEditorAtIndex3", ""),
    ("alt+4", "workbench.action.openEditorAtIndex4", ""),
    ("alt+5", "workbench.action.openEditorAtIndex5", ""),
    ("alt+6", "workbench.action.openEditorAtIndex6", ""),
    ("alt+7", "workbench.action.openEditorAtIndex7", ""),
    ("alt+8", "workbench.action.openEditorAtIndex8", ""),
    ("alt+9", "workbench.action.lastEditorInGroup", ""),
    ("ctrl+\\", "workbench.action.splitEditor", ""),
    ("ctrl+k ctrl+\\", "workbench.action.splitEditorOrthogonal", ""),
    ("ctrl+1", "workbench.action.focusFirstEditorGroup", ""),
    ("ctrl+2", "workbench.action.focusSecondEditorGroup", ""),
    ("ctrl+3", "workbench.action.focusThirdEditorGroup", ""),
    ("ctrl+k ctrl+left", "workbench.action.focusLeftGroup", ""),
    ("ctrl+k ctrl+right", "workbench.action.focusRightGroup", ""),
    ("ctrl+k ctrl+up", "workbench.action.focusAboveGroup", ""),
    ("ctrl+k ctrl+down", "workbench.action.focusBelowGroup", ""),
    ("ctrl+alt+right", "workbench.action.moveEditorToNextGroup", ""),
    ("ctrl+alt+left", "workbench.action.moveEditorToPreviousGroup", ""),
    ("ctrl+k ctrl+m", "workbench.action.toggleMaximizeEditorGroup", ""),
    ("ctrl+k ctrl+r", "workbench.files.action.showActiveFileInExplorer", ""),
    ("f6", "workbench.action.focusNextPart", ""),
    ("shift+f6", "workbench.action.focusPreviousPart", ""),
    (
        "escape",
        "yace.dismissNotification",
        "notificationVisible && !editorHasMultipleSelections && !editorHasSelection",
    ),
    // --- Editor ---------------------------------------------------------------
    ("ctrl+f", "actions.find", "editorFocus || findWidgetFocus"),
    ("ctrl+h", "editor.action.startFindReplaceAction", "(editorFocus || findWidgetFocus) && keyboardEnhanced"),
    ("f3", "editor.action.nextMatchFindAction", "editorFocus || findWidgetFocus"),
    ("shift+f3", "editor.action.previousMatchFindAction", "editorFocus || findWidgetFocus"),
    ("enter", "editor.action.nextMatchFindAction", "findWidgetFocus"),
    ("shift+enter", "editor.action.previousMatchFindAction", "findWidgetFocus"),
    ("alt+enter", "editor.action.selectAllMatches", "findWidgetFocus"),
    ("ctrl+shift+1", "editor.action.replaceOne", "findWidgetFocus"),
    ("ctrl+alt+enter", "editor.action.replaceAll", "findWidgetFocus"),
    ("alt+c", "toggleFindCaseSensitive", "findWidgetFocus"),
    ("alt+w", "toggleFindWholeWord", "findWidgetFocus"),
    ("alt+r", "toggleFindRegex", "findWidgetFocus"),
    ("alt+l", "toggleFindInSelection", "findWidgetFocus"),
    ("tab", "yace.find.toggleField", "findWidgetFocus"),
    ("escape", "closeFindWidget", "findWidgetFocus"),
    ("ctrl+shift+o", "workbench.action.gotoSymbol", ED),
    ("ctrl+space", "editor.action.triggerSuggest", ED),
    ("f2", "editor.action.rename", ED),
    ("f12", "editor.action.revealDefinition", ED),
    ("left", "cursorLeft", ED),
    ("right", "cursorRight", ED),
    ("up", "cursorUp", ED),
    ("down", "cursorDown", ED),
    ("shift+left", "cursorLeftSelect", ED),
    ("shift+right", "cursorRightSelect", ED),
    ("shift+up", "cursorUpSelect", ED),
    ("shift+down", "cursorDownSelect", ED),
    ("ctrl+left", "cursorWordLeft", ED),
    ("ctrl+right", "cursorWordRight", ED),
    ("ctrl+shift+left", "cursorWordLeftSelect", ED),
    ("ctrl+shift+right", "cursorWordRightSelect", ED),
    ("home", "cursorHome", ED),
    ("end", "cursorEnd", ED),
    ("shift+home", "cursorHomeSelect", ED),
    ("shift+end", "cursorEndSelect", ED),
    ("ctrl+home", "cursorTop", ED),
    ("ctrl+end", "cursorBottom", ED),
    ("ctrl+shift+home", "cursorTopSelect", ED),
    ("ctrl+shift+end", "cursorBottomSelect", ED),
    ("pageup", "cursorPageUp", ED),
    ("pagedown", "cursorPageDown", ED),
    ("shift+pageup", "cursorPageUpSelect", ED),
    ("shift+pagedown", "cursorPageDownSelect", ED),
    ("ctrl+up", "scrollLineUp", ED),
    ("ctrl+down", "scrollLineDown", ED),
    ("alt+pageup", "scrollPageUp", ED),
    ("alt+pagedown", "scrollPageDown", ED),
    ("backspace", "deleteLeft", ED),
    ("shift+backspace", "deleteLeft", ED),
    ("delete", "deleteRight", ED),
    ("ctrl+backspace", "deleteWordLeft", ED),
    ("ctrl+delete", "deleteWordRight", ED),
    ("enter", "yace.editor.newline", ED),
    ("shift+enter", "yace.editor.newline", ED),
    ("tab", "tab", ED),
    ("shift+tab", "outdent", ED),
    ("ctrl+]", "editor.action.indentLines", ED),
    ("ctrl+[", "editor.action.outdentLines", ED),
    ("ctrl+enter", "editor.action.insertLineAfter", ED),
    ("ctrl+shift+enter", "editor.action.insertLineBefore", ED),
    ("ctrl+a", "editor.action.selectAll", ED),
    ("ctrl+l", "expandLineSelection", ED),
    ("ctrl+z", "undo", ED),
    ("ctrl+y", "redo", ED),
    ("ctrl+shift+z", "redo", ED),
    ("ctrl+c", "editor.action.clipboardCopyAction", ED),
    ("ctrl+insert", "editor.action.clipboardCopyAction", ED),
    ("ctrl+x", "editor.action.clipboardCutAction", ED),
    ("shift+delete", "editor.action.clipboardCutAction", ED),
    ("ctrl+v", "editor.action.clipboardPasteAction", ED),
    ("shift+insert", "editor.action.clipboardPasteAction", ED),
    ("ctrl+d", "editor.action.addSelectionToNextFindMatch", ED),
    ("ctrl+shift+l", "editor.action.selectHighlights", ED),
    ("ctrl+shift+up", "editor.action.insertCursorAbove", ED),
    ("ctrl+shift+down", "editor.action.insertCursorBelow", ED),
    ("escape", "removeSecondaryCursors", "editorFocus && editorHasMultipleSelections"),
    ("escape", "cancelSelection", "editorFocus && editorHasSelection && !editorHasMultipleSelections"),
    ("escape", "closeFindWidget", "editorFocus && findWidgetVisible && !editorHasMultipleSelections"),
    ("alt+up", "editor.action.moveLinesUpAction", ED),
    ("alt+down", "editor.action.moveLinesDownAction", ED),
    ("shift+alt+up", "editor.action.copyLinesUpAction", ED),
    ("shift+alt+down", "editor.action.copyLinesDownAction", ED),
    ("ctrl+shift+alt+up", "editor.action.copyLinesUpAction", ED),
    ("ctrl+shift+alt+down", "editor.action.copyLinesDownAction", ED),
    ("ctrl+shift+k", "editor.action.deleteLines", ED),
    ("ctrl+/", "editor.action.commentLine", ED),
    ("shift+alt+a", "editor.action.blockComment", ED),
    ("ctrl+shift+[", "editor.fold", ED),
    ("ctrl+shift+]", "editor.unfold", ED),
    ("ctrl+k ctrl+l", "editor.toggleFold", ED),
    ("ctrl+k ctrl+0", "editor.foldAll", ED),
    ("ctrl+k ctrl+j", "editor.unfoldAll", ED),
    ("alt+z", "editor.action.toggleWordWrap", ""),
    ("ctrl+shift+\\", "editor.action.jumpToBracket", ED),
    ("ctrl+k ctrl+x", "editor.action.trimTrailingWhitespace", ED),
    // --- Explorer -------------------------------------------------------------
    ("up", "list.focusUp", "explorerFocus"),
    ("down", "list.focusDown", "explorerFocus"),
    ("left", "list.collapse", "explorerFocus"),
    ("right", "list.expand", "explorerFocus"),
    ("home", "list.focusFirst", "explorerFocus"),
    ("end", "list.focusLast", "explorerFocus"),
    ("pageup", "list.focusPageUp", "explorerFocus"),
    ("pagedown", "list.focusPageDown", "explorerFocus"),
    ("enter", "list.select", "explorerFocus"),
    ("space", "list.toggleExpandOrPreview", "explorerFocus && !explorerFilterActive"),
    ("escape", "list.clearFilter", "explorerFocus && explorerFilterActive"),
    ("backspace", "list.filterBackspace", "explorerFocus && explorerFilterActive"),
    ("f2", "renameFile", "explorerFocus"),
    ("delete", "deleteFile", "explorerFocus"),
    ("ctrl+alt+n", "explorer.newFile", "explorerFocus"),
    ("shift+alt+c", "copyFilePath", "explorerFocus"),
    ("ctrl+shift+alt+c", "copyRelativeFilePath", "explorerFocus"),
    ("shift+alt+f", "filesExplorer.findInFolder", "explorerFocus"),
    // --- Fallbacks for terminals without the enhanced keyboard protocol -------
    // Legacy terminals send Ctrl+\ as Ctrl+4, Ctrl+/ as Ctrl+7 and
    // Ctrl+Backspace as Ctrl+H, and cannot report Ctrl+Shift+<letter>.
    ("ctrl+4", "workbench.action.splitEditor", "!keyboardEnhanced"),
    ("ctrl+7", "editor.action.commentLine", "editorFocus && !keyboardEnhanced"),
    ("ctrl+h", "deleteWordLeft", "editorFocus && !keyboardEnhanced"),
    ("alt+e", "workbench.view.explorer", "!keyboardEnhanced"),
    ("alt+shift+f", "workbench.action.findInFiles", "!keyboardEnhanced && !explorerFocus"),
    ("alt+shift+h", "workbench.action.replaceInFiles", "!keyboardEnhanced"),
    ("alt+s", "workbench.action.files.saveAs", "!keyboardEnhanced"),
    ("alt+k", "editor.action.deleteLines", "editorFocus && !keyboardEnhanced"),
    ("alt+l", "editor.action.selectHighlights", "editorFocus && !keyboardEnhanced"),
    ("alt+t", "workbench.action.reopenClosedEditor", "!keyboardEnhanced"),
    ("alt+`", "workbench.action.terminal.toggleTerminal", "!keyboardEnhanced"),
    // Legacy terminals send Ctrl+` as Ctrl+Space (NUL); the two can't be told apart.
    ("ctrl+space", "workbench.action.terminal.toggleTerminal", "!keyboardEnhanced"),
    // --- Terminal (only these and the skip-shell list bypass the shell) ----
    ("ctrl+shift+c", "workbench.action.terminal.copySelection", "terminalFocus"),
    ("ctrl+shift+v", "workbench.action.terminal.paste", "terminalFocus"),
    ("shift+pageup", "workbench.action.terminal.scrollUpPage", "terminalFocus"),
    ("shift+pagedown", "workbench.action.terminal.scrollDownPage", "terminalFocus"),
    ("ctrl+shift+up", "workbench.action.terminal.scrollUp", "terminalFocus"),
    ("ctrl+shift+down", "workbench.action.terminal.scrollDown", "terminalFocus"),
    ("ctrl+pagedown", "workbench.action.terminal.focusNext", "terminalFocus"),
    ("ctrl+pageup", "workbench.action.terminal.focusPrevious", "terminalFocus"),
    // --- Language features (M7) ------------------------------------------
    ("ctrl+shift+space", "editor.action.triggerParameterHints", "editorFocus"),
    ("ctrl+k ctrl+i", "editor.action.showHover", "editorFocus"),
    ("ctrl+f12", "editor.action.goToImplementation", "editorFocus"),
    ("shift+f12", "editor.action.goToReferences", "editorFocus"),
    ("ctrl+.", "editor.action.quickFix", "editorFocus"),
    ("ctrl+shift+i", "editor.action.formatDocument", "editorFocus"),
    ("ctrl+k ctrl+f", "editor.action.formatSelection", "editorFocus"),
    ("f8", "editor.action.marker.nextInFiles", "editorFocus"),
    ("shift+f8", "editor.action.marker.prevInFiles", "editorFocus"),
    ("ctrl+shift+m", "workbench.actions.view.problems", ""),
    ("ctrl+t", "workbench.action.showAllSymbols", ""),
    // Legacy-terminal fallbacks for the Ctrl+Shift ones.
    ("alt+m", "workbench.actions.view.problems", "!keyboardEnhanced"),
    ("alt+o", "workbench.action.gotoSymbol", "!keyboardEnhanced"),
    ("alt+i", "editor.action.formatDocument", "editorFocus && !keyboardEnhanced"),
    ("ctrl+k ctrl+n", "yace.showNotifications", ""),
    // Popups (last, so they win while visible).
    ("escape", "editor.action.hideHover", "editorHoverVisible"),
    ("escape", "closeParameterHints", "parameterHintsVisible && !suggestWidgetVisible"),
    ("down", "selectNextSuggestion", "suggestWidgetVisible && editorFocus"),
    ("up", "selectPrevSuggestion", "suggestWidgetVisible && editorFocus"),
    ("pagedown", "selectNextPageSuggestion", "suggestWidgetVisible && editorFocus"),
    ("pageup", "selectPrevPageSuggestion", "suggestWidgetVisible && editorFocus"),
    ("enter", "acceptSelectedSuggestion", "suggestWidgetVisible && editorFocus"),
    ("tab", "acceptSelectedSuggestion", "suggestWidgetVisible && editorFocus"),
    ("escape", "hideSuggestWidget", "suggestWidgetVisible && editorFocus"),
];

/// Build the default keymap layer.
pub fn default_keymap() -> Keymap {
    let mut km = Keymap::new();
    for (keys, cmd, when) in DEFAULTS {
        km.add(keys, cmd, when, Source::Default);
    }
    km
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        keymap::{Resolution, Resolver},
        when::ContextSet,
    };

    #[test]
    fn defaults_parse_and_have_no_unintended_conflicts() {
        let km = default_keymap();
        assert!(km.bindings().len() > 100);
        assert!(km.conflicts().is_empty(), "{:?}", km.conflicts());
    }

    #[test]
    fn escape_routing_by_context() {
        let km = default_keymap();
        let mut r = Resolver::default();
        let key = "escape".parse().unwrap();
        let multi = ContextSet::new()
            .with(ctx::EDITOR_FOCUS)
            .with(ctx::EDITOR_HAS_MULTIPLE_SELECTIONS)
            .with(ctx::EDITOR_HAS_SELECTION);
        assert!(
            matches!(r.feed(&km, key, &multi), Resolution::Command { command, .. } if command == "removeSecondaryCursors")
        );
        let plain = ContextSet::new().with(ctx::EDITOR_FOCUS);
        assert_eq!(r.feed(&km, key, &plain), Resolution::Unbound);
    }

    #[test]
    fn legacy_ctrl_backtick_arrives_as_ctrl_space() {
        let km = default_keymap();
        let mut r = Resolver::default();
        let k = "ctrl+space".parse().unwrap();
        let legacy = ContextSet::new().with(ctx::EDITOR_FOCUS);
        assert!(
            matches!(r.feed(&km, k, &legacy), Resolution::Command { command, .. } if command == "workbench.action.terminal.toggleTerminal")
        );
        let enhanced = ContextSet::new().with(ctx::EDITOR_FOCUS).with(ctx::KEYBOARD_ENHANCED);
        assert!(
            matches!(r.feed(&km, k, &enhanced), Resolution::Command { command, .. } if command == "editor.action.triggerSuggest")
        );
        let t = ContextSet::new().with(ctx::TERMINAL_FOCUS);
        assert_eq!(
            km.lookup_single(k, &t).map(|b| b.command.as_str()),
            Some("workbench.action.terminal.toggleTerminal"),
            "toggles back out of the terminal"
        );
    }

    #[test]
    fn legacy_fallbacks_only_without_enhancement() {
        let km = default_keymap();
        let mut r = Resolver::default();
        let k = "ctrl+h".parse().unwrap();
        let legacy = ContextSet::new().with(ctx::EDITOR_FOCUS);
        assert!(matches!(r.feed(&km, k, &legacy), Resolution::Command { command, .. } if command == "deleteWordLeft"));
        let enhanced = ContextSet::new().with(ctx::EDITOR_FOCUS).with(ctx::KEYBOARD_ENHANCED);
        assert!(
            matches!(r.feed(&km, k, &enhanced), Resolution::Command { command, .. } if command == "editor.action.startFindReplaceAction")
        );
    }
}
