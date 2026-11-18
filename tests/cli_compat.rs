use std::path::Path;
use std::process::Command;

fn run(bin: &str, dir: &Path, env: &[(&str, &str)], args: &[&str]) -> String {
    let mut cmd = Command::new(bin);
    cmd.current_dir(dir).env("HOME", dir);
    for name in ["SAND_DB", "SEDIMENT_DB"] {
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
    let old = tmp.path().join("old").to_str().unwrap().to_string();
    let new = tmp.path().join("new").to_str().unwrap().to_string();
    // `stats` prints the database path first and loads no model when the database is missing
    let db = |env: &[(&str, &str)]| {
        let out = run(env!("CARGO_BIN_EXE_sand"), tmp.path(), env, &["stats"]);
        out.lines().next().unwrap().to_string()
    };
    assert_eq!(db(&[("SEDIMENT_DB", &old)]), format!("Database: {old}"));
    assert_eq!(
        db(&[("SEDIMENT_DB", &old), ("SAND_DB", &new)]),
        format!("Database: {new}")
    );
}
