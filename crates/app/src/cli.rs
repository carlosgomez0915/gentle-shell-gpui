//! Command line: `gentle-shell-gpui [FOLDER] [--cwd] [-p|--prompt TEXT]
//! [--open-url URL] [--diagnostics]`.

use std::path::{Path, PathBuf};

/// `gentle-shell-gpui [FOLDER] [--cwd] [-p|--prompt TEXT]`
pub struct Cli {
    pub workspace_id: Option<String>,
    pub folder: Option<PathBuf>,
    /// `--cwd`: without `FOLDER`, open the folder the app was launched in
    /// (what `gsd` passes). A plain launch doesn't: the Start menu or Finder
    /// start in some unrelated folder, and the last workspace is restored.
    pub here: bool,
    pub prompt: Option<String>,
    /// `--open-url URL`: show a page in the running app's browser.
    pub url: Option<String>,
}

pub const HELP: &str = "gentle-shell-gpui [FOLDER] [--cwd] [-p|--prompt TEXT] [--open-url URL] [--diagnostics]\ngentle-shell-gpui --workspace-id ID [--diagnostics]\n\n--workspace-id ID selects exactly one saved workspace (no folder, --cwd, prompt or URL).\nA missing ID fails on cold launch; a running app keeps its selection.";

pub fn parse_cli() -> Cli {
    parse_args(std::env::args_os().skip(1)).unwrap_or_else(|error| {
        eprintln!("gentle-shell-gpui: {error}");
        std::process::exit(2);
    })
}

pub fn parse_args(args: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Cli, String> {
    let mut cli = Cli {
        workspace_id: None,
        folder: None,
        here: false,
        prompt: None,
        url: None,
    };
    let mut content_option = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--workspace-id" {
            if cli.workspace_id.is_some() {
                return Err("--workspace-id may only be specified once".into());
            }
            let id = args
                .next()
                .and_then(|id| id.into_string().ok())
                .filter(|id| !id.trim().is_empty() && !id.starts_with('-'))
                .ok_or("--workspace-id expects an ID")?;
            cli.workspace_id = Some(id);
        } else if arg == "-p" || arg == "--prompt" {
            content_option = true;
            cli.prompt = args.next().map(|p| p.to_string_lossy().into_owned());
        } else if arg == "--cwd" {
            cli.here = true;
        } else if arg == "--open-url" || arg == "--url" {
            content_option = true;
            cli.url = Some(
                args.next()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            );
        } else if arg == crate::shared::diagnostics::FLAG {
            // Read by `diagnostics::enabled`.
        } else if cli.folder.is_none() {
            cli.folder = Some(PathBuf::from(arg));
        }
    }
    if cli.workspace_id.is_some() && (cli.folder.is_some() || cli.here || content_option) {
        return Err("--workspace-id cannot be combined with a folder, prompt or URL".into());
    }
    Ok(cli)
}

/// The folder the first tab opens: the CLI argument, else the folder the app
/// was launched in, else home (GUI launches start in `/`).
pub fn default_cwd() -> PathBuf {
    requested_folder(&parse_cli())
        .or_else(|| launch_folder(launch_dir(), pi_rpc::managed_install_root().as_deref()))
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The workspace folder this launch asks for: the folder argument, else with
/// `--cwd` the folder the app was launched in; `None` restores the last
/// workspace.
pub fn requested_folder(cli: &Cli) -> Option<PathBuf> {
    resolve_folder(cli.folder.clone(), cli.here, launch_dir())
}

/// `folder`, else with `here` the launch folder `launch`, as an existing
/// canonical folder (see [`cli_folder`]).
fn resolve_folder(folder: Option<PathBuf>, here: bool, launch: Option<PathBuf>) -> Option<PathBuf> {
    let folder = folder.or_else(|| (here && launch.is_some()).then(|| PathBuf::from(".")));
    cli_folder(folder, launch)
}

/// The CLI folder argument as an existing folder. A relative argument (`.`,
/// `myproject`) names a folder under `launch`, the folder the app was launched
/// in, not under the app's current folder, which startup moves (`main`, on
/// Windows).
fn cli_folder(arg: Option<PathBuf>, launch: Option<PathBuf>) -> Option<PathBuf> {
    arg.map(|p| match &launch {
        Some(launch) if p.is_relative() => launch.join(p),
        _ => p,
    })
    .filter(|p| p.is_dir())
    .and_then(|p| dunce::canonicalize(p).ok())
}

static LAUNCH_DIR: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();

/// Remember the folder the app was launched in, before startup moves the
/// app out of it (`main`, on Windows).
pub fn remember_launch_dir() {
    LAUNCH_DIR.get_or_init(|| std::env::current_dir().ok());
}

/// The folder the app was launched in: the one [`remember_launch_dir`] saw,
/// else the current one.
fn launch_dir() -> Option<PathBuf> {
    match LAUNCH_DIR.get() {
        Some(dir) => dir.clone(),
        None => std::env::current_dir().ok(),
    }
}

/// `launch` as the first tab's folder: not the root (where GUI launches
/// start) nor a folder inside gentle-ai's managed install (`managed_root`),
/// where a shortcut may start the app and where a tab would hold the
/// install open.
fn launch_folder(launch: Option<PathBuf>, managed_root: Option<&Path>) -> Option<PathBuf> {
    launch.filter(|p| p.parent().is_some() && !managed_root.is_some_and(|root| within(p, root)))
}

/// `path` is `root` or inside it, ignoring case (Windows paths).
fn within(path: &Path, root: &Path) -> bool {
    let lower = |path: &Path| -> Vec<String> {
        path.components()
            .map(|part| part.as_os_str().to_string_lossy().to_lowercase())
            .collect()
    };
    lower(path).starts_with(&lower(root))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn here_flag_uses_the_launch_folder_only_without_a_folder() {
        let base = std::env::temp_dir().join(format!("gsd-cli-test-{}", std::process::id()));
        let (here, there) = (base.join("here"), base.join("there"));
        std::fs::create_dir_all(&here).unwrap();
        std::fs::create_dir_all(&there).unwrap();
        let canon = |p: &Path| dunce::canonicalize(p).unwrap();

        // `gsd` in a folder: that folder.
        assert_eq!(
            resolve_folder(None, true, Some(here.clone())),
            Some(canon(&here))
        );
        // `gsd <folder>`: the folder wins over the launch folder.
        assert_eq!(
            resolve_folder(Some(there.clone()), true, Some(here.clone())),
            Some(canon(&there))
        );
        // A plain launch (Start menu, Finder) keeps restoring the last workspace.
        assert_eq!(resolve_folder(None, false, Some(here.clone())), None);
        // A folder that doesn't exist never falls back to the launch folder.
        assert_eq!(
            resolve_folder(Some(base.join("missing")), true, Some(here)),
            None
        );
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// A terminal launch without a folder argument opens the terminal's
    /// folder, as before the app moved off its launch folder at startup.
    #[test]
    fn a_terminal_launch_opens_the_terminals_folder() {
        let terminal = std::env::temp_dir().join("project");
        assert_eq!(
            launch_folder(Some(terminal.clone()), None),
            Some(terminal.clone())
        );
        let root = std::env::temp_dir().join("gentle-shell").join("install");
        assert_eq!(
            launch_folder(Some(terminal.clone()), Some(&root)),
            Some(terminal)
        );
    }

    #[test]
    fn a_gui_launch_at_the_root_opens_home() {
        let root = std::env::temp_dir()
            .ancestors()
            .last()
            .unwrap()
            .to_path_buf();
        assert_eq!(launch_folder(Some(root), None), None);
    }

    /// A shortcut that starts the app inside gentle-ai's managed install
    /// doesn't make that install the first tab's folder.
    #[test]
    fn a_launch_inside_the_managed_install_opens_home() {
        let root = std::env::temp_dir().join("gentle-shell").join("install");
        let programs = root
            .join("home")
            .join("AppData")
            .join("Local")
            .join("Programs")
            .join("Gentle Shell");
        assert_eq!(launch_folder(Some(programs), Some(&root)), None);
        assert_eq!(launch_folder(Some(root.clone()), Some(&root)), None);
    }

    /// A relative folder argument names a folder under the launch folder,
    /// even after startup moved the app's current folder elsewhere.
    #[test]
    fn a_relative_folder_argument_resolves_against_the_launch_folder() {
        let launch = std::env::temp_dir().join(format!("gentle-cli-launch-{}", std::process::id()));
        let project = launch.join("myproject");
        std::fs::create_dir_all(&project).unwrap();
        let expected = dunce::canonicalize(&project).unwrap();
        let resolved = cli_folder(Some(PathBuf::from("myproject")), Some(launch.clone()));
        let dot = cli_folder(Some(PathBuf::from(".")), Some(project.clone()));
        std::fs::remove_dir_all(&launch).unwrap();
        assert_eq!(resolved, Some(expected.clone()));
        assert_eq!(dot, Some(expected));
    }

    /// An absolute folder argument opens as given, as before.
    #[test]
    fn an_absolute_folder_argument_opens_as_given() {
        let folder = dunce::canonicalize(std::env::temp_dir()).unwrap();
        let elsewhere = Some(PathBuf::from("Z:\nowhere"));
        assert_eq!(cli_folder(Some(folder.clone()), elsewhere), Some(folder));
        assert_eq!(cli_folder(None, Some(std::env::temp_dir())), None);
    }
}
