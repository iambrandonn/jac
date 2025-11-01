use predicates::prelude::*;
use serde_json::Value;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/wrapper")
        .join(name)
}

fn temp_output_path(dir: &TempDir, name: &str) -> PathBuf {
    dir.path().join(name)
}

// Helper to parse NDJSON output into Vec<Value>
fn parse_ndjson(path: &Path) -> Result<Vec<Value>, Box<dyn Error>> {
    let contents = fs::read_to_string(path)?;
    Ok(contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect())
}

#[test]
fn basic_pointer_extraction() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let jac_path = temp_output_path(&dir, "output.jac");
    let ndjson_path = temp_output_path(&dir, "output.ndjson");

    // Compress with pointer wrapper
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("simple_envelope.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-pointer",
            "/data",
        ])
        .assert()
        .success();

    // Decompress
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "unpack",
            jac_path.to_str().unwrap(),
            "-o",
            ndjson_path.to_str().unwrap(),
            "--ndjson",
        ])
        .assert()
        .success();

    // Verify output
    let records = parse_ndjson(&ndjson_path)?;
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["id"], 1);
    assert_eq!(records[0]["name"], "alice");
    assert_eq!(records[1]["id"], 2);
    assert_eq!(records[1]["name"], "bob");

    Ok(())
}

#[test]
fn nested_pointer_extraction() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let jac_path = temp_output_path(&dir, "output.jac");
    let ndjson_path = temp_output_path(&dir, "output.ndjson");

    // Compress with nested pointer
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("nested_envelope.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-pointer",
            "/api/v1/results",
        ])
        .assert()
        .success();

    // Decompress
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "unpack",
            jac_path.to_str().unwrap(),
            "-o",
            ndjson_path.to_str().unwrap(),
            "--ndjson",
        ])
        .assert()
        .success();

    // Verify output
    let records = parse_ndjson(&ndjson_path)?;
    assert_eq!(records.len(), 3);
    assert_eq!(records[0]["userId"], 1);
    assert_eq!(records[0]["status"], "active");

    Ok(())
}

#[test]
fn pointer_to_root() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let input_path = temp_output_path(&dir, "input.json");
    let jac_path = temp_output_path(&dir, "output.jac");
    let ndjson_path = temp_output_path(&dir, "output.ndjson");

    // Create an array at root
    fs::write(&input_path, r#"[{"id":1},{"id":2}]"#)?;

    // Compress with empty pointer (root)
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            input_path.to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-pointer",
            "",
        ])
        .assert()
        .success();

    // Decompress
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "unpack",
            jac_path.to_str().unwrap(),
            "-o",
            ndjson_path.to_str().unwrap(),
            "--ndjson",
        ])
        .assert()
        .success();

    // Verify output
    let records = parse_ndjson(&ndjson_path)?;
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["id"], 1);

    Ok(())
}

#[test]
fn pointer_to_object() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let jac_path = temp_output_path(&dir, "output.jac");
    let ndjson_path = temp_output_path(&dir, "output.ndjson");

    // Compress with pointer to object (should emit single record)
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("object_target.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-pointer",
            "/user",
        ])
        .assert()
        .success();

    // Decompress
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "unpack",
            jac_path.to_str().unwrap(),
            "-o",
            ndjson_path.to_str().unwrap(),
            "--ndjson",
        ])
        .assert()
        .success();

    // Verify output
    let records = parse_ndjson(&ndjson_path)?;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["id"], 1);
    assert_eq!(records[0]["name"], "alice");

    Ok(())
}

#[test]
fn empty_array_handling() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let jac_path = temp_output_path(&dir, "output.jac");
    let ndjson_path = temp_output_path(&dir, "output.ndjson");

    // Compress empty array
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("empty_array.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-pointer",
            "/data",
        ])
        .assert()
        .success();

    // Decompress
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "unpack",
            jac_path.to_str().unwrap(),
            "-o",
            ndjson_path.to_str().unwrap(),
            "--ndjson",
        ])
        .assert()
        .success();

    // Verify output is empty
    let records = parse_ndjson(&ndjson_path)?;
    assert_eq!(records.len(), 0);

    Ok(())
}

#[test]
fn missing_pointer_error() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let jac_path = temp_output_path(&dir, "output.jac");

    // Should fail with pointer not found
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("missing_pointer.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-pointer",
            "/data",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));

    Ok(())
}

#[test]
fn null_target_rejection() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let jac_path = temp_output_path(&dir, "output.jac");

    // Should fail with wrong type
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("null_target.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-pointer",
            "/data",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("wrong type").or(predicate::str::contains("null")));

    Ok(())
}

#[test]
fn scalar_target_rejection() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let jac_path = temp_output_path(&dir, "output.jac");

    // Should fail with wrong type
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("scalar_target.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-pointer",
            "/value",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("type mismatch").or(predicate::str::contains("number")));

    Ok(())
}

#[test]
fn escaped_keys() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let jac_path = temp_output_path(&dir, "output.jac");
    let ndjson_path = temp_output_path(&dir, "output.ndjson");

    // Compress with escaped pointer (/ becomes ~1, ~ becomes ~0)
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("escaped_keys.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-pointer",
            "/field~1name/field~0name",
        ])
        .assert()
        .success();

    // Decompress
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "unpack",
            jac_path.to_str().unwrap(),
            "-o",
            ndjson_path.to_str().unwrap(),
            "--ndjson",
        ])
        .assert()
        .success();

    // Verify output
    let records = parse_ndjson(&ndjson_path)?;
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["id"], 1);
    assert_eq!(records[0]["value"], "test");

    Ok(())
}

#[test]
fn invalid_pointer_syntax() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let jac_path = temp_output_path(&dir, "output.jac");

    // Pointer must start with / (unless empty for root)
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("simple_envelope.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-pointer",
            "data", // Missing leading /
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("invalid").or(predicate::str::contains("must start with")),
        );

    Ok(())
}

#[test]
fn depth_limit_validation() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let jac_path = temp_output_path(&dir, "output.jac");

    // Depth must be > 0 and <= 10
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("simple_envelope.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-pointer",
            "/data",
            "--wrapper-pointer-depth",
            "0",
        ])
        .assert()
        .failure();

    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("simple_envelope.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-pointer",
            "/data",
            "--wrapper-pointer-depth",
            "20",
        ])
        .assert()
        .failure();

    Ok(())
}

#[test]
fn buffer_limit_validation() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let jac_path = temp_output_path(&dir, "output.jac");

    // Buffer must be > 0 and <= 128M
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("simple_envelope.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-pointer",
            "/data",
            "--wrapper-pointer-buffer",
            "0",
        ])
        .assert()
        .failure();

    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("simple_envelope.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-pointer",
            "/data",
            "--wrapper-pointer-buffer",
            "256M",
        ])
        .assert()
        .failure();

    Ok(())
}

#[test]
fn ndjson_without_wrapper_unchanged() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let input_path = temp_output_path(&dir, "input.ndjson");
    let jac_path = temp_output_path(&dir, "output.jac");
    let output_path = temp_output_path(&dir, "output.ndjson");

    // Create NDJSON input
    fs::write(&input_path, "{\"id\":1}\n{\"id\":2}\n")?;

    // Compress without wrapper
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            input_path.to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    // Decompress
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "unpack",
            jac_path.to_str().unwrap(),
            "-o",
            output_path.to_str().unwrap(),
            "--ndjson",
        ])
        .assert()
        .success();

    // Verify output matches input
    let records = parse_ndjson(&output_path)?;
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["id"], 1);
    assert_eq!(records[1]["id"], 2);

    Ok(())
}
