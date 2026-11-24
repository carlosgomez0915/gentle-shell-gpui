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
