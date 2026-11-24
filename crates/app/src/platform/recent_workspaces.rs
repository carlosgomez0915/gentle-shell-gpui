//! Windows Jump Lists. No shell effects for isolated app instances.
#[cfg(any(windows, test))]
use std::ffi::OsStr;

#[cfg(any(windows, test))]
fn isolated(value: Option<&OsStr>) -> bool {
    // Match data_dir::app: an empty override still uses production storage.
    value.is_some_and(|value| !value.is_empty())
}

pub fn app_id() -> Option<&'static str> {
    #[cfg(windows)]
    if isolated(std::env::var_os("GENTLE_SHELL_GPUI_DATA_DIR").as_deref()) {
        return None;
    }
    Some("dev.gentle-shell.gpui")
}

pub fn init() {
    #[cfg(windows)]
    windows::init();
}

pub fn publish(state: &crate::features::workspaces::SavedState) {
    #[cfg(windows)]
    windows::publish(state);
    #[cfg(not(windows))]
    let _ = state;
}

#[cfg(any(windows, test))]
fn project<'a>(
    state: &'a crate::features::workspaces::SavedState,
    removed: &[String],
    slots: usize,
) -> Vec<&'a crate::features::workspaces::SavedWorkspace> {
    let mut rows: Vec<_> = state.workspaces.iter().enumerate().collect();
    rows.sort_by_key(|(i, w)| (std::cmp::Reverse(w.last_active), *i != state.current, *i));
    rows.into_iter()
        .filter(|(_, w)| {
            w.id.len() == 32
                && w.id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                && !w.root.as_os_str().is_empty()
                && !removed.contains(&format!("--workspace-id {}", w.id))
        })
        .take(slots)
        .map(|(_, w)| w)
        .collect()
}

#[cfg(windows)]
mod windows {
    use super::*;
    use ::windows::Win32::Foundation::PROPERTYKEY;
    use ::windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use ::windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    };
    use ::windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
    use ::windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use ::windows::Win32::UI::Shell::{
        DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW,
        SetCurrentProcessExplicitAppUserModelID, ShellLink,
    };
    use ::windows::core::GUID;
    use ::windows::core::{Interface, PCWSTR};
    use std::sync::{OnceLock, mpsc};

    const APP_ID: &str = "dev.gentle-shell.gpui";
    static WORKER: OnceLock<Option<mpsc::Sender<crate::features::workspaces::SavedState>>> =
        OnceLock::new();
    fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        s.as_ref().encode_wide().chain(Some(0)).collect()
    }
    pub(super) fn init() {
        if isolated(std::env::var_os("GENTLE_SHELL_GPUI_DATA_DIR").as_deref()) {
            return;
        }
        let identity =
            unsafe { SetCurrentProcessExplicitAppUserModelID(PCWSTR(wide(APP_ID).as_ptr())) };
        WORKER.get_or_init(|| {
            if let Err(error) = identity {
                eprintln!("Jump List identity: {error}");
                return None;
            }
            let (send, receive) = mpsc::channel();
            std::thread::Builder::new()
                .name("jump-list".into())
                .spawn(move || {
                    // Every COM object is created, used and released on this one STA.
                    if unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_err() {
                        return;
                    }
                    struct Apartment;
                    impl Drop for Apartment {
                        fn drop(&mut self) {
                            unsafe { CoUninitialize() };
                        }
                    }
                    let _apartment = Apartment;
                    for state in receive {
                        if let Err(error) = unsafe { update(&state) } {
                            eprintln!("Jump List: {error}");
                        }
                    }
                })
                .ok()?;
            Some(send)
        });
    }
    pub(super) fn publish(state: &crate::features::workspaces::SavedState) {
        if isolated(std::env::var_os("GENTLE_SHELL_GPUI_DATA_DIR").as_deref()) {
            return;
        }
        if let Some(Some(sender)) = WORKER.get() {
            let _ = sender.send(state.clone());
        }
    }
    struct Transaction(ICustomDestinationList, bool);
    impl Drop for Transaction {
        fn drop(&mut self) {
            if !self.1 {
                let _ = unsafe { self.0.AbortList() };
            }
        }
    }
    unsafe fn update(
        state: &crate::features::workspaces::SavedState,
    ) -> ::windows::core::Result<()> {
        // Never substitute a folder or an empty removed list after a COM error.
        let exe = std::env::current_exe()?;
        unsafe {
            let list: ICustomDestinationList =
                CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER)?;
            list.SetAppID(PCWSTR(wide(APP_ID).as_ptr()))?;
            let mut slots = 0;
            let removed: IObjectArray = list.BeginList(&mut slots)?;
            let mut transaction = Transaction(list, false);
            let mut exclusions = Vec::new();
            for i in 0..removed.GetCount()? {
                let link: IShellLinkW = removed.GetAt(i)?;
                let mut args = [0u16; 1024];
                link.GetArguments(&mut args)?;
                let end = args.iter().position(|c| *c == 0).unwrap_or(args.len());
                exclusions.push(String::from_utf16_lossy(&args[..end]));
            }
            let collection: IObjectCollection =
                CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)?;
            for workspace in project(state, &exclusions, slots as usize) {
                let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
                link.SetPath(PCWSTR(wide(&exe).as_ptr()))?;
                link.SetArguments(PCWSTR(
                    wide(format!("--workspace-id {}", workspace.id)).as_ptr(),
                ))?;
                let title = format!("{} — {}", workspace.name, workspace.root.display());
                link.SetDescription(PCWSTR(wide(&title).as_ptr()))?;
                let properties: IPropertyStore = link.cast()?;
                // PKEY_Title, required for shell links in custom categories.
                let key = PROPERTYKEY {
                    fmtid: GUID::from_u128(0xf29f85e0_4ff9_1068_ab91_08002b27b3d9),
                    pid: 2,
                };
                let value = PROPVARIANT::from(title.as_str());
                properties.SetValue(&key, &value)?;
                properties.Commit()?;
                collection.AddObject(&link)?;
            }
            let array: IObjectArray = collection.cast()?;
            if array.GetCount()? > 0 {
                transaction
                    .0
                    .AppendCategory(PCWSTR(wide("Workspaces").as_ptr()), &array)?;
            }
            transaction.0.CommitList()?;
            transaction.1 = true;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn isolation_matches_storage_override() {
        assert!(!isolated(None));
        assert!(!isolated(Some(OsStr::new(""))));
        assert!(isolated(Some(OsStr::new("isolated"))));
    }
    #[test]
    fn projection_filters_removed_caps_and_keeps_current_then_index_ties() {
        use crate::features::workspaces::SavedState;
        let mut state: SavedState = serde_json::from_value(serde_json::json!({
            "current": 2,
            "workspaces": [
                {"id": "11111111111111111111111111111111", "name": "first", "root": "one", "last_active": 4},
                {"id": "22222222222222222222222222222222", "name": "newest", "root": "two", "last_active": 5},
                {"id": "33333333333333333333333333333333", "name": "current", "root": "three", "last_active": 4},
                {"id": "44444444444444444444444444444444", "name": "last", "root": "four", "last_active": 4},
                {"id": "invalid", "name": "invalid", "root": "five", "last_active": 9}
            ]
        })).unwrap();
        assert_eq!(
            project(&state, &[], 9)
                .iter()
                .map(|w| w.name.as_str())
                .collect::<Vec<_>>(),
            ["newest", "current", "first", "last"]
        );
        let removed = vec![format!("--workspace-id {}", state.workspaces[1].id)];
        assert_eq!(
            project(&state, &removed, 2)
                .iter()
                .map(|w| w.name.as_str())
                .collect::<Vec<_>>(),
            ["current", "first"]
        );
        assert!(project(&state, &[], 0).is_empty());
        state.workspaces[1].root.clear();
        assert_eq!(project(&state, &[], 1)[0].name, "current");
    }
}
