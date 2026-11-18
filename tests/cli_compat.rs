//! sand stays a drop-in replacement for sediment 0.5.1.

use std::path::Path;
use std::process::Command;

const SAND: &str = env!("CARGO_BIN_EXE_sand");
const SEDIMENT: &str = env!("CARGO_BIN_EXE_sediment");

/// Run a binary with HOME in `home` and every SAND_*/SEDIMENT_* setting removed except `env`.
fn run(bin: &str, home: &Path, env: &[(&str, &str)], args: &[&str]) -> String {
    let mut cmd = Command::new(bin);
    cmd.current_dir(home).env("HOME", home);
    for name in ["DB", "EMBEDDING_MODEL", "PROJECT_ID"] {
        cmd.env_remove(format!("SAND_{name}"))
            .env_remove(format!("SEDIMENT_{name}"));
    }
    let output = cmd.envs(env.iter().copied()).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{bin} {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn test_sediment_executable_matches_sand() {
    let tmp = tempfile::TempDir::new().unwrap();
    for args in [&["--version"][..], &["--help"], &["stats"]] {
        assert_eq!(
            run(SEDIMENT, tmp.path(), &[], args),
            run(SAND, tmp.path(), &[], args),
            "args: {args:?}"
        );
    }
}

#[test]
fn test_sediment_settings_are_fallbacks() {
    let tmp = tempfile::TempDir::new().unwrap();
    let home = tmp.path();
    let old_db = home.join("old").to_str().unwrap().to_string();
    let new_db = home.join("new").to_str().unwrap().to_string();
    let flag_db = home.join("flag").to_str().unwrap().to_string();
    let stats_db = |env: &[(&str, &str)], args: &[&str]| {
        let out = run(SAND, home, env, &[&["stats"], args].concat());
        out.lines().next().unwrap().to_string()
    };

    assert_eq!(
        stats_db(&[("SEDIMENT_DB", &old_db)], &[]),
        format!("Database: {old_db}")
    );
    let both = [("SEDIMENT_DB", old_db.as_str()), ("SAND_DB", &new_db)];
    assert_eq!(stats_db(&both, &[]), format!("Database: {new_db}"));
    assert_eq!(
        stats_db(&both, &["--db", &flag_db]),
        format!("Database: {flag_db}")
    );

    let project_id = |env: &[(&str, &str)]| {
        let out = run(SAND, home, env, &["--json", "project"]);
        serde_json::from_str::<serde_json::Value>(&out).unwrap()["project_id"].clone()
    };
    assert_eq!(project_id(&[("SEDIMENT_PROJECT_ID", "old")]), "old");
    assert_eq!(
        project_id(&[("SEDIMENT_PROJECT_ID", "old"), ("SAND_PROJECT_ID", "new")]),
        "new"
    );
    // an empty SAND_PROJECT_ID is set: it selects global context
    assert!(project_id(&[("SEDIMENT_PROJECT_ID", "old"), ("SAND_PROJECT_ID", "")]).is_null());
}
