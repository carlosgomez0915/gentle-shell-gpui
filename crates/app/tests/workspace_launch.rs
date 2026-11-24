//! Public CLI checks use a fresh data directory, never the installed app.
#[path = "../src/cli.rs"]
mod cli;
mod shared {
    pub mod diagnostics {
        pub const FLAG: &str = "--diagnostics";
    }
}

#[test]
fn public_parser_preserves_existing_inputs_and_selects_exact_id() {
    let parse =
        |args: &[&str]| cli::parse_args(args.iter().map(|arg| std::ffi::OsString::from(*arg)));
    let ordinary = parse(&["project", "-p", "hello", "--url", "https://example.com"]).unwrap();
    assert_eq!(ordinary.folder, Some("project".into()));
    assert_eq!(ordinary.prompt.as_deref(), Some("hello"));
    assert_eq!(ordinary.url.as_deref(), Some("https://example.com"));
    assert!(ordinary.workspace_id.is_none());
    let selected = parse(&["--workspace-id", "sibling-b", "--diagnostics"]).unwrap();
    assert_eq!(selected.workspace_id.as_deref(), Some("sibling-b"));
    assert!(selected.folder.is_none());
    assert!(selected.prompt.is_none());
    assert!(selected.url.is_none());
    let _ = cli::parse_cli as fn() -> cli::Cli;
    let _ = cli::default_cwd as fn() -> std::path::PathBuf;
    let _ = cli::remember_launch_dir as fn();
    assert!(cli::HELP.contains("--workspace-id ID"));
}

#[test]
fn gsd_and_explicit_cwd_forms_keep_folder_resolution() {
    let expected = dunce::canonicalize(std::env::current_dir().unwrap()).unwrap();
    for args in [
        vec!["--cwd"],
        vec!["--cwd", "."],
        vec!["."],
        vec!["--cwd", "--prompt", "hello"],
    ] {
        let parsed =
            cli::parse_args(args.iter().map(|arg| std::ffi::OsString::from(*arg))).unwrap();
        assert_eq!(cli::requested_folder(&parsed), Some(expected.clone()));
    }
    let plain = cli::parse_args([]).unwrap();
    assert_eq!(cli::requested_folder(&plain), None);
    let missing =
        cli::parse_args(["--cwd", "nonexistent-workspace-launch-folder"].map(Into::into)).unwrap();
    assert_eq!(cli::requested_folder(&missing), None);
}

use std::process::{Command, Output};
struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "workspace-launch-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_gentle-shell-gpui"))
            .args(args)
            .env("GENTLE_SHELL_GPUI_DATA_DIR", &self.0)
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn workspace_selector_is_forwarded_without_cold_lookup() {
    for (args, reply, accepted) in [
        (vec!["--workspace-id", "sibling-b"], "ok\n", false),
        (
            vec!["--workspace-id", "sibling-b"],
            "workspace-id-v1\n",
            true,
        ),
        (vec!["--workspace-id", "sibling-b"], "", false),
        (vec!["--workspace-id", "sibling-b"], "other\n", false),
        (vec![], "ok\n", true),
    ] {
        check_peer(&args, reply, accepted);
    }
}
fn check_peer(args: &[&str], reply: &'static str, accepted: bool) {
    use std::io::{BufRead, Write};
    let fixture = Fixture::new();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    std::fs::write(fixture.0.join("instance.json"), serde_json::json!({
        "port": listener.local_addr().unwrap().port(), "token": "fixture", "pid": std::process::id()
    }).to_string()).unwrap();
    let record = std::fs::read(fixture.0.join("instance.json")).unwrap();
    std::fs::write(fixture.0.join("workspaces.json"), "unchanged").unwrap();
    let receiver = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut line = String::new();
        std::io::BufReader::new(&stream)
            .read_line(&mut line)
            .unwrap();
        stream.write_all(reply.as_bytes()).unwrap();
        serde_json::from_str::<serde_json::Value>(&line).unwrap()
    });
    let output = fixture.run(args);
    assert_eq!(output.status.code(), Some(if accepted { 0 } else { 2 }));
    assert!(output.stdout.is_empty());
    let stderr = if accepted {
        ""
    } else {
        "gentle-shell-gpui: the running app did not confirm workspace selection support; quit it and retry with this version\n"
    };
    assert_eq!(String::from_utf8(output.stderr).unwrap(), stderr);
    let message = receiver.join().unwrap();
    assert_eq!(message["token"], "fixture");
    let selector = if args.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::json!("sibling-b")
    };
    assert_eq!(message["request"]["workspace_id"], selector);
    for field in ["folder", "prompt", "url", "device"] {
        assert!(message["request"][field].is_null());
    }
    assert_eq!(
        std::fs::read(fixture.0.join("instance.json")).unwrap(),
        record
    );
    assert_eq!(
        std::fs::read_to_string(fixture.0.join("workspaces.json")).unwrap(),
        "unchanged"
    );
    assert_eq!(std::fs::read_dir(&fixture.0).unwrap().count(), 2);
}

#[test]
fn workspace_selector_rejects_conflicts_before_startup() {
    let fixture = Fixture::new();
    for (args, message) in [
        (vec!["--workspace-id"], "--workspace-id expects an ID"),
        (vec!["--workspace-id", ""], "--workspace-id expects an ID"),
        (
            vec!["--workspace-id", "one", "--workspace-id", "two"],
            "--workspace-id may only be specified once",
        ),
    ] {
        let output = fixture.run(&args);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!("gentle-shell-gpui: {message}\n")
        );
        assert_eq!(std::fs::read_dir(&fixture.0).unwrap().count(), 0);
    }
    for args in [
        vec!["--workspace-id", "one", "."],
        vec![".", "--workspace-id", "one"],
        vec!["--workspace-id", "one", "--prompt", "hello"],
        vec!["--open-url", "https://example.com", "--workspace-id", "one"],
        vec!["--cwd", "--workspace-id", "one"],
        vec!["--workspace-id", "one", "--cwd"],
        vec![
            "--cwd",
            "nonexistent-workspace-launch-folder",
            "--workspace-id",
            "one",
        ],
    ] {
        let output = fixture.run(&args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            "gentle-shell-gpui: --workspace-id cannot be combined with a folder, prompt or URL\n"
        );
        assert_eq!(std::fs::read_dir(&fixture.0).unwrap().count(), 0);
    }
}

#[test]
fn stale_workspace_selector_fails_without_starting_the_app() {
    let fixture = Fixture::new();
    let state = r#"{"current":0,"workspaces":[{"id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","name":"A","root":"same-root"},{"id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","name":"B","root":"same-root"}]}"#;
    std::fs::write(fixture.0.join("workspaces.json"), state).unwrap();
    let output = fixture.run(&["--workspace-id", "missing"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "gentle-shell-gpui: workspace ID not found: missing\n"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.0.join("workspaces.json")).unwrap(),
        state
    );
}
