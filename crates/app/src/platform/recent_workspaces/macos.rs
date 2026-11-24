//! Transient Dock menu, owned by GPUI's existing application delegate.
use super::dock_request;
#[cfg(target_os = "macos")]
use super::{DockUpdates, dock_rows};
#[cfg(target_os = "macos")]
use gpui_kit::MenuItem;
use gpui_kit::{Action, App};
use std::rc::Rc;
#[cfg(target_os = "macos")]
use std::sync::{Mutex, OnceLock};

#[derive(Clone, PartialEq, Action)]
#[action(namespace = gentle, no_json)]
struct OpenRecentWorkspace {
    id: String,
}

#[cfg(target_os = "macos")]
struct Bridge {
    updates: Mutex<DockUpdates>,
    wake: async_channel::Sender<()>,
    receive: async_channel::Receiver<()>,
}

#[cfg(target_os = "macos")]
fn bridge() -> &'static Bridge {
    static BRIDGE: OnceLock<Bridge> = OnceLock::new();
    BRIDGE.get_or_init(|| {
        let (wake, receive) = async_channel::bounded(1);
        Bridge {
            updates: Mutex::new(DockUpdates::default()),
            wake,
            receive,
        }
    })
}

/// Called only after the existing store successfully persists a snapshot.
/// One pending metadata projection and one wake, regardless of save frequency.
#[cfg(target_os = "macos")]
pub(super) fn publish(state: &crate::features::workspaces::SavedState) {
    let bridge = bridge();
    bridge.updates.lock().unwrap().push(dock_rows(state));
    let _ = bridge.wake.try_send(());
}

fn register_action(
    cx: &mut App,
    open: impl Fn(crate::platform::instance::OpenRequest, &mut App) + 'static,
) {
    let open = Rc::new(open);
    cx.on_action(move |action: &OpenRecentWorkspace, cx| {
        let open = open.clone();
        let request = dock_request(&action.id);
        // GPUI removes the dispatching window from App during action handling.
        // Wait until it is restored before the callback updates that same handle.
        cx.defer(move |cx| open(request, cx));
    });
}

#[cfg(target_os = "macos")]
pub fn install(
    cx: &mut App,
    open: impl Fn(crate::platform::instance::OpenRequest, &mut App) + 'static,
) {
    register_action(cx, open);
    let receive = bridge().receive.clone();
    cx.spawn(async move |cx| {
        while receive.recv().await.is_ok() {
            let rows = bridge().updates.lock().unwrap().take_changed();
            if let Some(rows) = rows {
                cx.update(|cx| {
                    cx.set_dock_menu(
                        rows.into_iter()
                            .map(|row| {
                                MenuItem::action(row.label, OpenRecentWorkspace { id: row.id })
                            })
                            .collect(),
                    );
                });
            }
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::{OpenRecentWorkspace, register_action};
    use gpui_kit::{Context, IntoElement, Render, TestAppContext, Window, div};
    use std::{cell::RefCell, rc::Rc};

    struct Host;

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    #[gpui::test]
    fn dock_action_can_update_its_dispatching_window(cx: &mut TestAppContext) {
        let (_, cx) = cx.add_window_view(|_, _| Host);
        let observed = Rc::new(RefCell::new(Vec::new()));
        let received = observed.clone();
        let id = "22222222222222222222222222222222";
        cx.update(|window, cx| {
            let handle = window.window_handle();
            register_action(cx, move |request, cx| {
                let updated = handle.update(cx, |_, _, _| ()).is_ok();
                received.borrow_mut().push((request, updated));
            });
            // Exercise GPUI's real action dispatch, not a direct callback call.
            window.dispatch_action(Box::new(OpenRecentWorkspace { id: id.into() }), cx);
        });
        cx.run_until_parked();
        assert_eq!(
            *observed.borrow(),
            vec![(
                crate::platform::instance::OpenRequest {
                    workspace_id: Some(id.into()),
                    ..Default::default()
                },
                true
            )]
        );
    }
}
