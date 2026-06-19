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
//! Supported keys (mirroring lightjj's revision-list keymap exactly):
//! - `j` / `ArrowDown`  — move selection down one row (clamped at the bottom)
//! - `k` / `ArrowUp`    — move selection up one row (clamped at the top)
//! - `1` / `2` / `3`    — switch the active view (Revisions / Branches / Merge)
//!
//! Deliberately ABSENT (lightjj binds none of these on the revision list):
//! - There is NO list-jump: lightjj's revision-list nav handler (`navKey` in
//!   App.svelte) binds only `j`/`k`; `Home`/`End` are unbound and `g`/`G` are
//!   NOT jumps — `g` is the global git-mode prefix (App.svelte handleGlobalKeys
//!   `case 'g'` → openModal('git')).
//! - `4`/`5` toggle the Oplog/Evolog bottom *drawers* in lightjj
//!   (App.svelte handleGlobalKeys cases '4'/'5' → toggleOplog/toggleEvolog).
//!   jjscratch's [`UiState`]/[`View`] has no field to represent an open drawer
//!   yet, so they are intentionally omitted here rather than misrepresented as
//!   view switches. (The toolbar renders their [4]/[5] hints in src/ui.rs.)

use crate::model::Snapshot;
use crate::ui::{UiState, View};

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
        // Unknown key: no-op, selection unchanged.
        _ => return false,
    }

    // Keep `selected` in range even if it was somehow out of bounds coming in
    // (e.g. a snapshot shrank). This never widens it past the last row.
    if len != 0 && state.selected > last {
        state.selected = last;
    }

    state.selected != before
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
}
