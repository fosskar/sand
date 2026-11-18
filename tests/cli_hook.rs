use std::path::Path;
use std::process::Command;

use serde_json::json;

/// Run `sand hook recall` for one prompt and return its stdout.
fn hook(dir: &Path, session_id: &str, prompt: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sand"))
        .env("SAND_DB", dir.join("data"))
        .env("XDG_RUNTIME_DIR", dir)
        .args(["hook", "recall", "--max-tries", "2"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let input = json!({ "prompt": prompt, "cwd": dir, "session_id": session_id });
    serde_json::to_writer(child.stdin.take().unwrap(), &input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "sand hook recall failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn test_hook_recall_counts_prompts_without_match_up_to_max_tries() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path();
    let marker = dir.join("sand-recall").join("s_1");

    // the database does not exist, so no prompt matches
    for tries in ["1", "2", "2"] {
        assert_eq!(hook(dir, "s/1", "which shell does the user use"), "");
        assert_eq!(std::fs::read_to_string(&marker).unwrap().trim(), tries);
    }
}

#[test]
#[ignore] // requires model download
fn test_hook_recall_prints_once_per_session() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path();
    let note = "the user prefers fish as the interactive shell";
    let stored = Command::new(env!("CARGO_BIN_EXE_sand"))
        .env("SAND_DB", dir.join("data"))
        .current_dir(dir)
        .args(["store", "--scope", "global", note])
        .output()
        .unwrap();
    assert!(stored.status.success());

    let first = hook(dir, "a", "which interactive shell does the user prefer");
    assert!(first.starts_with("<recalled_memories>\n"), "{first}");
    assert!(first.contains(note), "{first}");
    assert_eq!(
        hook(dir, "a", "which interactive shell does the user prefer"),
        ""
    );

    // a session whose first prompt matched nothing recalls again on the next one
    assert_eq!(hook(dir, "b", "rust lifetime elision rules"), "");
    assert!(hook(dir, "b", "which interactive shell does the user prefer").contains(note));
}
