use assert_cmd::Command;
use predicates::prelude::*;
use std::error::Error;
use tempfile::TempDir;

#[test]
fn pack_with_unknown_format_flag_fails() -> Result<(), Box<dyn Error>> {
    let dir = TempDir::new()?;
    let output = dir.path().join("out.jac");

    Command::cargo_bin("jac")?
        .args([
            "pack",
            "testdata/conformance.ndjson",
            "-o",
            output.to_str().unwrap(),
            "--format",
            "yaml",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument"));
    Ok(())
}

#[test]
fn cat_with_invalid_block_range_fails() -> Result<(), Box<dyn Error>> {
    let dir = TempDir::new()?;
    let jac_path = dir.path().join("input.jac");

    Command::cargo_bin("jac")?
        .args([
            "pack",
            "testdata/conformance.ndjson",
            "-o",
            jac_path.to_str().unwrap(),
            "--ndjson",
        ])
        .assert()
        .success();

    Command::cargo_bin("jac")?
        .args([
            "cat",
            jac_path.to_str().unwrap(),
            "--field",
            "user",
            "--blocks",
            "5-3",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("start (5) > end (3)"));

    Command::cargo_bin("jac")?
        .args([
            "cat",
            jac_path.to_str().unwrap(),
            "--field",
            "user",
            "--blocks",
            "abc",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid block range syntax"));

    Ok(())
}
