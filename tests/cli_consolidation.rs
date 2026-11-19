use std::process::Command;

#[test]
#[ignore] // requires model download
fn test_cli_store_merges_a_duplicate_before_it_exits() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sand = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_sand"))
            .current_dir(tmp.path())
            .env("SAND_DB", tmp.path().join("data"))
            .arg("--json")
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "sand {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };

    let note = "the deploy script runs every night at 02:30";
    sand(&["store", note]);
    sand(&["store", note]);

    assert_eq!(sand(&["list", "--scope", "all"])["count"], 1);
}
