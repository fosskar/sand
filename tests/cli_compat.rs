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
