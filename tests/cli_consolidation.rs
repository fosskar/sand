use std::process::Command;

fn sand(dir: &std::path::Path, args: &[&str]) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_sand"))
        .current_dir(dir)
        .env("SAND_DB", dir.join("data"))
        .env("SEDIMENT_DB", dir.join("data"))
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
}

#[test]
#[ignore] // requires model download
fn test_cli_store_merges_a_duplicate_before_it_exits() {
    let tmp = tempfile::TempDir::new().unwrap();
    sand(
        tmp.path(),
        &["store", "the deploy script runs every night at 02:30"],
    );
    sand(
        tmp.path(),
        &["store", "  The deploy script\nruns every NIGHT at 02:30 "],
    );

    assert_eq!(sand(tmp.path(), &["list", "--scope", "all"])["count"], 1);
}

#[test]
#[ignore] // requires model download
fn test_cli_store_keeps_two_different_notes_above_merge_similarity() {
    let tmp = tempfile::TempDir::new().unwrap();
    // cosine 0.969 with all-MiniLM-L6-v2, above the old 0.95 merge threshold; the rates differ
    let first = "Implemented API rate limiting with token bucket algorithm at 1000 requests per hour per API key to prevent abuse.";
    let second = "Implemented rate limiting with token bucket algorithm at 100 requests per minute per API key to prevent abuse.";
    sand(tmp.path(), &["store", first]);
    sand(tmp.path(), &["store", second]);

    assert_eq!(sand(tmp.path(), &["list", "--scope", "all"])["count"], 2);
}
