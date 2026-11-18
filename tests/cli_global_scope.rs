use std::path::Path;
use std::process::Command;

use serde_json::Value;

const PROJECT_A: &str = "00000000000000000000000000000000000000aa";
const PROJECT_B: &str = "00000000000000000000000000000000000000bb";

fn sediment(dir: &Path, project_id: &str, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_sediment"))
        .current_dir(dir)
        .env("SEDIMENT_DB", dir.join("data"))
        .env("SEDIMENT_PROJECT_ID", project_id)
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "sediment {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn list(dir: &Path, project_id: &str, scope: &str) -> Vec<(String, String)> {
    sediment(dir, project_id, &["list", "--scope", scope])["items"]
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
    let dir = tmp.path();

    let old_id = sediment(
        dir,
        PROJECT_A,
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
        list(dir, PROJECT_A, "global"),
        vec![(old_id.clone(), "global".to_string())]
    );
    assert!(list(dir, PROJECT_A, "project").is_empty());

    let new_id = sediment(
        dir,
        PROJECT_A,
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
        list(dir, PROJECT_A, "global"),
        vec![(new_id.clone(), "global".to_string())]
    );
    assert!(list(dir, PROJECT_A, "project").is_empty());
    assert_eq!(list(dir, PROJECT_A, "all").len(), 1);

    // another project sees the item as global: no cross-project flag, may forget it
    let recall = sediment(
        dir,
        PROJECT_B,
        &["recall", "the user prefers tabs over spaces, width 4"],
    );
    let hit = recall["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == new_id.as_str())
        .expect("recall should return the global item");
    assert!(hit.get("cross_project").is_none(), "got: {}", hit);

    sediment(dir, PROJECT_B, &["forget", &new_id]);
    assert!(list(dir, PROJECT_A, "all").is_empty());
}
