//! Pure keyboard router for jjscratch, mirroring lightjj's navigation semantics.
//!
//! This is the keyboard model that makes jjscratch *drivable*: the same key
//! tokens that drive the real lightjj web app (via headless Chrome) are routed
//! here to mutate [`UiState`] identically, so a single interaction script can be
//! replayed against both apps for pixel-parity comparison.
//!
//! The router is deliberately PURE: it takes a key name, the current
//! [`UiState`], and the read-only [`Snapshot`] it is navigating over, mutates the
//! state, and returns whether the *selection* changed (so the caller knows to
//! recompute the diff that follows the cursor). It performs no I/O and owns no
//! state of its own, which keeps it trivially unit-testable.
//!
//! Supported keys (mirroring lightjj's global + revision-list keymap):
//! - `j` / `ArrowDown`  — move selection down one row (clamped at the bottom)
//! - `k` / `ArrowUp`    — move selection up one row (clamped at the top)
//! - `1` / `2` / `3`    — switch the active view (Revisions / Branches / Merge)
//! - `t`                — toggle the theme polarity (dark ⇄ light)
//! - `4`                — toggle the Oplog bottom drawer (switches to Revisions
//!                        first, as lightjj `switchToLogView(); toggleOplog()`)
//! - `5`                — toggle the Evolog bottom drawer (likewise)
//! - `cmd+k` / `ctrl+k` — open the command palette overlay
//!
//! While the palette is open, all keys route into it: printable single-char
//! keys (and the `Backspace`/`Space` names) edit `palette_query`, and `Escape`
//! closes the palette. This mirrors lightjj's `handleGlobalOverrides` (Cmd+K →
//! `paletteOpen = true`) + the palette input's own keydown handling.
//!
//! lightjj bindings cited (App.svelte):
//! - `handleGlobalKeys` `case 't'` → toggleTheme.
//! - `handleGlobalKeys` `case '4'` → switchToLogView(); toggleOplog().
//! - `handleGlobalKeys` `case '5'` → switchToLogView(); toggleEvolog().
//! - `handleGlobalOverrides` `e.key === 'k'` under metaKey/ctrlKey → paletteOpen.
//!
//! Deliberately ABSENT: there is NO list-jump — lightjj's `navKey` binds only
//! `j`/`k`; `Home`/`End` are unbound and `g`/`G` are NOT jumps (`g` is the
//! global git-mode prefix, `openModal('git')`).
//!
//! ## Cmd+K token
//!
//! The cross-driver harness dispatches the palette shortcut as the lowercase
//! token **`"cmd+k"`** (with `"ctrl+k"` accepted as an alias, since lightjj's
//! `cmdKey` renders `Ctrl` off-mac). The `drive` bin emits whichever the script
//! contains; both map to the same toggle here.

use crate::model::Snapshot;
use crate::ui::{Theme, UiState, View};

/// Route a single key press into `state`, navigating over `snapshot.nodes`.
///
/// `key` is a logical key name. Both the single-character form (`"j"`, `"1"`)
/// and the spelled-out browser `KeyboardEvent.key` form (`"ArrowDown"`,
/// `"ArrowUp"`) are accepted so the same token works when dispatched to a
/// browser or applied natively.
///
/// Returns `true` iff `state.selected` changed as a result (i.e. the cursor
/// moved to a different revision), so the caller can reload the diff that
/// follows the selection. View switches (`1`/`2`/`3`) return `false` — they do
/// not move the revision cursor.
pub fn handle_key(key: &str, state: &mut UiState, snapshot: &Snapshot) -> bool {
    // While the command palette is open it captures every key (lightjj's modal
    // input has focus): typing edits the query, Escape closes it. This runs
    // BEFORE the global keymap so `t`/`4`/`j` etc. are typed into the query
    // rather than firing app actions.
    if state.palette_open {
        route_palette_key(key, state);
        return false;
    }

    let len = snapshot.nodes.len();
    // With no rows there is nothing to select; view switches still apply.
    let last = len.saturating_sub(1);
    let before = state.selected;

    match key {
        // lightjj App.svelte navKey: `j`/ArrowDown → select(selectedIndex + 1),
        // guarded by `selectedIndex < revisions.length - 1` (clamped at bottom).
        "j" | "ArrowDown" => {
            if len != 0 {
                state.selected = (state.selected + 1).min(last);
            }
        }
        // lightjj App.svelte navKey: `k`/ArrowUp → select(selectedIndex - 1),
        // guarded by `selectedIndex > 0` (clamped at top).
        "k" | "ArrowUp" => {
            state.selected = state.selected.saturating_sub(1);
        }
        // lightjj App.svelte handleGlobalKeys case '1' → switchToLogView().
        "1" => {
            state.active_view = View::Revisions;
            return false;
        }
        // lightjj App.svelte handleGlobalKeys case '2' → switchToBranchesView().
        "2" => {
            state.active_view = View::Branches;
            return false;
        }
        // lightjj App.svelte handleGlobalKeys case '3' → switchToMergeView().
        "3" => {
            state.active_view = View::Merge;
            return false;
        }
        // lightjj App.svelte handleGlobalKeys case 't' → toggleTheme().
        "t" => {
            state.theme = match state.theme {
                Theme::Dark => Theme::Light,
                Theme::Light => Theme::Dark,
            };
            return false;
        }
        // lightjj handleGlobalKeys case '4' → switchToLogView(); toggleOplog().
        // The drawers are gated on the log view in lightjj, so switch first.
        "4" => {
            state.active_view = View::Revisions;
            state.oplog_open = !state.oplog_open;
            if state.oplog_open {
                state.evolog_open = false; // one bottom drawer at a time
            }
            return false;
        }
        // lightjj handleGlobalKeys case '5' → switchToLogView(); toggleEvolog().
        "5" => {
            state.active_view = View::Revisions;
            state.evolog_open = !state.evolog_open;
            if state.evolog_open {
                state.oplog_open = false;
            }
            return false;
        }
        // lightjj handleGlobalOverrides: Cmd/Ctrl+K → closeModals(); paletteOpen.
        "cmd+k" | "ctrl+k" => {
            state.palette_open = true;
            state.palette_query.clear();
            return false;
        }
        // Unknown key: no-op, selection unchanged.
        _ => return false,
    }

    // Keep `selected` in range even if it was somehow out of bounds coming in
    // (e.g. a snapshot shrank out from under the cursor, or a stale `UiState` was
    // restored against a different snapshot). An empty snapshot pins it to 0
    // (there is no row to select); otherwise it is clamped to the last row,
    // never widened past it. This is the single authority that upholds the
    // `selected < nodes.len()` (or 0) invariant the renderer relies on, no
    // matter which navigation key ran above — in particular `k` on an empty
    // snapshot with a stale nonzero `selected` would otherwise leave it nonzero.
    if len == 0 {
        state.selected = 0;
    } else if state.selected > last {
        state.selected = last;
    }

    state.selected != before
}

/// Route a key into the open command palette: edit `palette_query` or close.
///
/// Mirrors lightjj's palette input handling: `Escape` closes (clearing the
/// query), `Backspace` deletes the last char, and any single printable
/// character (including a literal space, or the `"Space"`/`" "` key names)
/// appends to the query. View/global keys are intentionally swallowed while the
/// palette is focused — they become query text, not app actions.
fn route_palette_key(key: &str, state: &mut UiState) {
    match key {
        "Escape" | "Esc" => {
            state.palette_open = false;
            state.palette_query.clear();
        }
        "Backspace" => {
            state.palette_query.pop();
        }
        "Space" | " " => {
            state.palette_query.push(' ');
        }
        // A single printable character (the common case: typing a query).
        other if other.chars().count() == 1 => {
            let c = other.chars().next().unwrap();
            if !c.is_control() {
                state.palette_query.push(c);
            }
        }
        // Multi-char key names (Enter, ArrowDown, cmd+k, …): no-op for now.
        // (Enter would run the active command once mutations land.)
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::mock;

    fn state_at(selected: usize) -> UiState {
        UiState { selected, ..UiState::default() }
    }

    #[test]
    fn j_moves_down_and_clamps_at_bottom() {
        let snap = mock::snapshot();
        let last = snap.nodes.len() - 1;
        let mut st = state_at(0);

        // Each `j` moves down one and reports a change...
        assert!(handle_key("j", &mut st, &snap));
        assert_eq!(st.selected, 1);

        // ...until we reach the bottom, where it clamps and reports no change.
        st.selected = last;
        assert!(!handle_key("j", &mut st, &snap));
        assert_eq!(st.selected, last);
    }

    #[test]
    fn k_moves_up_and_clamps_at_top() {
        let snap = mock::snapshot();
        let mut st = state_at(2);

        assert!(handle_key("k", &mut st, &snap));
        assert_eq!(st.selected, 1);

        st.selected = 0;
        assert!(!handle_key("k", &mut st, &snap));
        assert_eq!(st.selected, 0);
    }

    #[test]
    fn arrow_keys_alias_jk() {
        let snap = mock::snapshot();
        let mut st = state_at(1);
        assert!(handle_key("ArrowDown", &mut st, &snap));
        assert_eq!(st.selected, 2);
        assert!(handle_key("ArrowUp", &mut st, &snap));
        assert_eq!(st.selected, 1);
    }

    #[test]
    fn view_keys_switch_view_without_moving_cursor() {
        let snap = mock::snapshot();
        let mut st = state_at(2);

        assert!(!handle_key("2", &mut st, &snap));
        assert_eq!(st.active_view, View::Branches);
        assert_eq!(st.selected, 2, "view switch must not move the cursor");

        assert!(!handle_key("3", &mut st, &snap));
        assert_eq!(st.active_view, View::Merge);

        assert!(!handle_key("1", &mut st, &snap));
        assert_eq!(st.active_view, View::Revisions);
    }

    #[test]
    fn list_jump_keys_are_unbound() {
        // lightjj's revision list has NO Home/End/g/G jump; they are no-ops here.
        let snap = mock::snapshot();
        let mut st = state_at(1);
        for key in ["Home", "End", "g", "G"] {
            assert!(!handle_key(key, &mut st, &snap), "{key} must not move cursor");
            assert_eq!(st.selected, 1, "{key} must not move cursor");
            assert_eq!(st.active_view, View::Revisions, "{key} must not switch view");
        }
    }

    #[test]
    fn unknown_key_is_noop() {
        let snap = mock::snapshot();
        let mut st = state_at(1);
        assert!(!handle_key("x", &mut st, &snap));
        assert_eq!(st.selected, 1);
        assert_eq!(st.active_view, View::Revisions);
    }

    #[test]
    fn empty_snapshot_does_not_panic() {
        let mut snap = mock::snapshot();
        snap.nodes.clear();
        let mut st = state_at(0);
        assert!(!handle_key("j", &mut st, &snap));
        assert!(!handle_key("k", &mut st, &snap));
        assert_eq!(st.selected, 0);
    }

    #[test]
    fn empty_snapshot_with_stale_selection_is_pinned_to_zero() {
        // CHAOS regression: a stale `UiState` (selection left over from a larger
        // snapshot) must be coerced back to 0 the moment a navigation key runs
        // against an empty snapshot — otherwise `k`/ArrowUp would decrement a
        // bogus index and leave `selected` nonzero, violating the renderer's
        // `selected < nodes.len()` (or 0) invariant.
        let mut snap = mock::snapshot();
        snap.nodes.clear();
        let mut st = state_at(5); // stale: there are no rows
        handle_key("k", &mut st, &snap);
        assert_eq!(st.selected, 0, "k on an empty snapshot must pin selection to 0");
        st.selected = 9;
        handle_key("j", &mut st, &snap);
        assert_eq!(st.selected, 0, "j on an empty snapshot must pin selection to 0");
        st.selected = 3;
        handle_key("ArrowUp", &mut st, &snap);
        assert_eq!(st.selected, 0, "ArrowUp on an empty snapshot must pin selection to 0");
    }

    #[test]
    fn stale_overflow_selection_is_clamped_to_last_row() {
        // A selection past the end of a (non-empty) snapshot is clamped to the
        // last row on the next navigation key, never read out of bounds.
        let snap = mock::snapshot();
        let last = snap.nodes.len() - 1;
        let mut st = state_at(usize::MAX); // wildly stale
        handle_key("k", &mut st, &snap);
        assert_eq!(st.selected, last, "overflow selection clamps to the last row");
    }

    #[test]
    fn jk_spam_past_both_bounds_stays_in_range() {
        // CHAOS regression: hammering j then k far past both ends must keep the
        // selection a valid index the whole time and never wrap or overflow.
        let snap = mock::snapshot();
        let last = snap.nodes.len() - 1;
        let mut st = state_at(0);
        for _ in 0..1000 {
            handle_key("j", &mut st, &snap);
            assert!(st.selected <= last, "j must never escape the bottom bound");
        }
        assert_eq!(st.selected, last);
        for _ in 0..1000 {
            handle_key("k", &mut st, &snap);
            assert!(st.selected <= last, "k must keep a valid index");
        }
        assert_eq!(st.selected, 0, "k spam lands and clamps at the top");
    }

    #[test]
    fn palette_backspace_storm_on_empty_query_is_a_noop() {
        // CHAOS regression: Backspace on an already-empty query must not panic
        // or corrupt the (UTF-8) query string.
        let snap = mock::snapshot();
        let mut st = state_at(0);
        handle_key("cmd+k", &mut st, &snap);
        for _ in 0..50 {
            handle_key("Backspace", &mut st, &snap);
        }
        assert!(st.palette_query.is_empty());
        assert!(st.palette_open, "backspacing an empty query must not close the palette");
    }

    #[test]
    fn palette_unicode_typing_and_backspace_stays_valid_utf8() {
        // CHAOS regression: typing multi-byte chars then backspacing must delete
        // whole `char`s (String::pop), never split a UTF-8 boundary.
        let snap = mock::snapshot();
        let mut st = state_at(0);
        handle_key("cmd+k", &mut st, &snap);
        for k in ["é", "中", "ß", "💥"] {
            handle_key(k, &mut st, &snap);
        }
        assert_eq!(st.palette_query, "é中ß💥");
        // Backspace removes one whole char at a time.
        handle_key("Backspace", &mut st, &snap);
        assert_eq!(st.palette_query, "é中ß");
        // Control chars are rejected, not appended.
        handle_key("\u{0}", &mut st, &snap);
        assert_eq!(st.palette_query, "é中ß", "control chars must not enter the query");
    }

    #[test]
    fn junk_keys_are_inert() {
        // CHAOS regression: unrecognized / empty / multi-char / unicode key
        // tokens must be pure no-ops (no panic, no state change) when the palette
        // is closed.
        let snap = mock::snapshot();
        let mut st = state_at(2);
        for k in ["", "qwerty", "💥", "\u{0}", "\n", "F1", "Shift", "cmd+j", "中"] {
            assert!(!handle_key(k, &mut st, &snap), "junk key {k:?} must not move cursor");
        }
        assert_eq!(st.selected, 2);
        assert_eq!(st.active_view, View::Revisions);
        assert_eq!(st.theme, Theme::Dark);
        assert!(!st.oplog_open && !st.evolog_open && !st.palette_open);
    }

    #[test]
    fn drawers_stay_mutually_exclusive_under_toggle_storm() {
        // CHAOS regression: any interleaving of 4/5 must never leave both bottom
        // drawers open at once.
        let snap = mock::snapshot();
        let mut st = state_at(0);
        for k in ["4", "5", "4", "4", "5", "5", "4", "5"] {
            handle_key(k, &mut st, &snap);
            assert!(
                !(st.oplog_open && st.evolog_open),
                "at most one bottom drawer open at a time (after {k})"
            );
        }
    }

    #[test]
    fn t_toggles_theme_without_moving_cursor() {
        // lightjj handleGlobalKeys case 't' → toggleTheme().
        let snap = mock::snapshot();
        let mut st = state_at(2);
        assert_eq!(st.theme, Theme::Dark, "default polarity is dark");
        assert!(!handle_key("t", &mut st, &snap));
        assert_eq!(st.theme, Theme::Light);
        assert_eq!(st.selected, 2, "theme toggle must not move the cursor");
        assert!(!handle_key("t", &mut st, &snap));
        assert_eq!(st.theme, Theme::Dark, "second press flips back");
    }

    #[test]
    fn key4_toggles_oplog_and_switches_to_revisions() {
        // lightjj: switchToLogView(); toggleOplog().
        let snap = mock::snapshot();
        let mut st = state_at(2);
        st.active_view = View::Branches;
        assert!(!handle_key("4", &mut st, &snap));
        assert!(st.oplog_open, "4 opens the oplog drawer");
        assert_eq!(st.active_view, View::Revisions, "4 switches to the log view first");
        assert_eq!(st.selected, 2, "drawer toggle must not move the cursor");
        // Toggling again closes it.
        assert!(!handle_key("4", &mut st, &snap));
        assert!(!st.oplog_open);
    }

    #[test]
    fn key5_toggles_evolog_and_is_mutually_exclusive_with_oplog() {
        let snap = mock::snapshot();
        let mut st = state_at(1);
        handle_key("4", &mut st, &snap); // open oplog
        assert!(st.oplog_open && !st.evolog_open);
        handle_key("5", &mut st, &snap); // open evolog → closes oplog
        assert!(st.evolog_open && !st.oplog_open, "only one bottom drawer at a time");
        assert_eq!(st.active_view, View::Revisions);
    }

    #[test]
    fn cmd_k_opens_palette_and_typing_edits_query() {
        let snap = mock::snapshot();
        let mut st = state_at(0);
        // Cmd+K (and the Ctrl+K alias) open the palette.
        assert!(!handle_key("cmd+k", &mut st, &snap));
        assert!(st.palette_open);
        assert!(st.palette_query.is_empty());

        // While open, keys route into the query — even view/global keys like
        // `2`, `t`, `j` become query text, not app actions.
        for k in ["t", "h", "e", "2"] {
            handle_key(k, &mut st, &snap);
        }
        assert_eq!(st.palette_query, "the2");
        assert_eq!(st.active_view, View::Revisions, "view keys are swallowed by the palette");
        assert_eq!(st.theme, Theme::Dark, "t is swallowed by the palette");

        // Backspace deletes; Space inserts a literal space.
        handle_key("Backspace", &mut st, &snap);
        handle_key("Space", &mut st, &snap);
        assert_eq!(st.palette_query, "the ");

        // Escape closes and clears.
        assert!(!handle_key("Escape", &mut st, &snap));
        assert!(!st.palette_open);
        assert!(st.palette_query.is_empty());
    }

    #[test]
    fn ctrl_k_is_a_palette_alias() {
        let snap = mock::snapshot();
        let mut st = state_at(0);
        assert!(!handle_key("ctrl+k", &mut st, &snap));
        assert!(st.palette_open);
    }

    #[test]
    fn palette_swallows_navigation_keys() {
        // j/k must not move the cursor while the palette is focused.
        let snap = mock::snapshot();
        let mut st = state_at(2);
        st.palette_open = true;
        assert!(!handle_key("j", &mut st, &snap));
        assert_eq!(st.selected, 2, "palette captures j");
        assert_eq!(st.palette_query, "j", "j becomes query text");
    }
}
