use std::path::Path;
use std::process::Command;

use serde_json::Value;

/// Run `sand --json` with `project` as the working directory; the store is next to it.
fn sand(project: &Path, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_sand"))
        .current_dir(project)
        .env("SAND_DB", project.parent().unwrap().join("data"))
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
    serde_json::from_slice(&output.stdout).unwrap()
}

fn list(project: &Path, scope: &str) -> Vec<(String, String)> {
    sand(project, &["list", "--scope", scope])["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["id"].as_str().unwrap().to_string(),
                i["scope"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
#[ignore] // requires model download
fn test_cli_store_global_scope_is_global() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (project_a, project_b) = (tmp.path().join("a"), tmp.path().join("b"));
    std::fs::create_dir(&project_a).unwrap();
    std::fs::create_dir(&project_b).unwrap();

    let old_id = sand(
        &project_a,
        &[
            "store",
            "--scope",
            "global",
            "the user prefers tabs over spaces",
        ],
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        list(&project_a, "global"),
        vec![(old_id.clone(), "global".to_string())]
    );
    assert!(list(&project_a, "project").is_empty());

    let new_id = sand(
        &project_a,
        &[
            "store",
            "--scope",
            "global",
            "--replace",
            &old_id,
            "the user prefers tabs over spaces, width 4",
        ],
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        list(&project_a, "global"),
        vec![(new_id.clone(), "global".to_string())]
    );
    assert!(list(&project_a, "project").is_empty());
    assert_eq!(list(&project_a, "all").len(), 1);

    // another project sees the item as global: no cross-project flag, may forget it
    let recall = sand(
        &project_b,
        &["recall", "the user prefers tabs over spaces, width 4"],
    );
    let hit = recall["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == new_id.as_str())
        .expect("recall should return the global item");
    assert!(hit.get("cross_project").is_none(), "got: {}", hit);

    sand(&project_b, &["forget", &new_id]);
    assert!(list(&project_a, "all").is_empty());
}
