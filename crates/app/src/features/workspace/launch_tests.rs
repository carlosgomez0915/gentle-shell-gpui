//! Deterministic production-boundary tests: no windows, backends or user data.
use crate::features::workspaces::{self, SavedState};

fn siblings() -> SavedState {
    serde_json::from_value(serde_json::json!({
        "current": 0,
        "workspaces": [
            {"id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "name": "First", "root": "shared-root"},
            {"id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "name": "Second", "root": "shared-root"}
        ]
    }))
    .unwrap()
}

const SECOND: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[test]
fn cold_explicit_launch_selects_only_the_same_root_second_workspace() {
    let before = siblings();
    let mut loads = 0;
    let selected = workspaces::select_launch(Some(SECOND), || {
        loads += 1;
        before.clone()
    })
    .unwrap()
    .unwrap();
    assert_eq!(loads, 1);
    assert_eq!(selected.current, 1);
    assert_eq!(selected.workspaces, before.workspaces);
    assert_eq!(selected.workspaces.len(), 2);
}

#[test]
fn cold_unknown_id_rejects_the_snapshot_without_altering_it() {
    let before = siblings();
    let mut loads = 0;
    let result = workspaces::select_launch(Some("deleted"), || {
        loads += 1;
        before.clone()
    });
    assert_eq!(loads, 1);
    assert_eq!(result.unwrap_err(), "workspace ID not found: deleted");
    assert_eq!(before, siblings());
}

#[test]
fn ordinary_launch_does_not_preload_or_override_saved_selection() {
    let selected =
        workspaces::select_launch(None, || panic!("ordinary startup loads in Workspace::new"))
            .unwrap();
    assert!(selected.is_none());
}

#[test]
fn warm_selector_targets_second_sibling_and_rejects_unknown_without_fallback() {
    let saved = siblings();
    let before = saved.clone();
    let ids = || {
        saved
            .workspaces
            .iter()
            .map(|workspace| workspace.id.as_str())
    };
    assert_eq!(workspaces::workspace_index(SECOND, ids()), Ok(1));
    assert_eq!(workspaces::workspace_index(SECOND, ids()), Ok(1));
    assert_eq!(
        workspaces::workspace_index("deleted", ids()),
        Err("workspace ID not found: deleted".into())
    );
    assert_eq!(saved, before);
    // Selection is an index, not a new workspace: repeated requests cannot
    // append anything. GPUI switch/focus and notice rendering are not exercised.
    assert_eq!(saved.workspaces.len(), 2);
}
