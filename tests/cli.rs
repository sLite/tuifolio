use std::{
    path::Path,
    process::{Command, Output},
};

const REMOVED_COMMANDS: [&str; 6] = [
    "summary",
    "holding",
    "missing-prices",
    "sync-prices",
    "add-price",
    "base",
];

fn cli(path: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tuifolio"))
        .arg("--store")
        .arg(path)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn help_lists_only_web_and_does_not_open_the_datastore() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("data/store.json");
    for args in [vec!["--help"], vec!["web", "--help"]] {
        let output = cli(&path, &args);
        assert!(output.status.success());
        let help = String::from_utf8(output.stdout).unwrap();
        assert!(help.contains("web"));
        for command in REMOVED_COMMANDS {
            assert!(!help.contains(command), "obsolete command in help: {help}");
        }
        assert!(!path.parent().unwrap().exists());
    }
    let help = cli(&path, &["web", "--help"]);
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("--port"));
    assert!(help.contains("--price-refresh-seconds"));
}

#[test]
fn removed_commands_are_rejected_before_creating_a_datastore_or_lock() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("data/store.json");
    for command in REMOVED_COMMANDS {
        let output = cli(&path, &[command]);
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("unrecognized subcommand"));
        assert!(!path.parent().unwrap().exists());
    }
}

#[test]
fn removed_commands_do_not_read_or_rewrite_an_existing_datastore() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.json");
    let original = b"not valid datastore JSON\n";
    std::fs::write(&path, original).unwrap();
    for command in REMOVED_COMMANDS {
        let output = cli(&path, &[command]);
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("unrecognized subcommand"));
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(!directory.path().join("store.json.lock").exists());
    }
}
