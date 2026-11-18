use std::path::Path;
use std::process::Command;

fn run(bin: &str, dir: &Path, env: &[(&str, &str)], args: &[&str]) -> String {
    let mut cmd = Command::new(bin);
    cmd.current_dir(dir).env("HOME", dir);
    for name in [
        "SAND_DB",
        "SEDIMENT_DB",
        "SAND_PROJECT_ID",
        "SEDIMENT_PROJECT_ID",
    ] {
        cmd.env_remove(name);
    }
    let output = cmd.envs(env.iter().copied()).args(args).output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn test_sediment_help_matches_sand() {
    let tmp = tempfile::TempDir::new().unwrap();
    assert_eq!(
        run(env!("CARGO_BIN_EXE_sediment"), tmp.path(), &[], &["--help"]),
        run(env!("CARGO_BIN_EXE_sand"), tmp.path(), &[], &["--help"])
    );
}

#[test]
fn test_sediment_setting_used_only_when_sand_setting_unset() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project_id = |env: &[(&str, &str)]| {
        let out = run(
            env!("CARGO_BIN_EXE_sand"),
            tmp.path(),
            env,
            &["--json", "project"],
        );
        serde_json::from_str::<serde_json::Value>(&out).unwrap()["project_id"].clone()
    };
    assert_eq!(project_id(&[("SEDIMENT_PROJECT_ID", "old")]), "old");
    assert!(project_id(&[("SEDIMENT_PROJECT_ID", "old"), ("SAND_PROJECT_ID", "")]).is_null());
}
