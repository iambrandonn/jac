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
fn basic_array_with_headers() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let jac_path = temp_output_path(&dir, "output.jac");
    let ndjson_path = temp_output_path(&dir, "output.ndjson");

    // Compress with array-headers wrapper
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("array_headers_basic.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-array-headers",
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

    // Verify first record
    assert_eq!(records[0]["id"].as_i64().unwrap(), 1);
    assert_eq!(records[0]["name"].as_str().unwrap(), "Alice");
    assert_eq!(records[0]["age"].as_i64().unwrap(), 30);

    // Verify second record
    assert_eq!(records[1]["id"].as_i64().unwrap(), 2);
    assert_eq!(records[1]["name"].as_str().unwrap(), "Bob");
    assert_eq!(records[1]["age"].as_i64().unwrap(), 25);

    // Verify third record
    assert_eq!(records[2]["id"].as_i64().unwrap(), 3);
    assert_eq!(records[2]["name"].as_str().unwrap(), "Carol");
    assert_eq!(records[2]["age"].as_i64().unwrap(), 35);

    Ok(())
}

#[test]
fn array_headers_mixed_types() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let jac_path = temp_output_path(&dir, "output.jac");
    let ndjson_path = temp_output_path(&dir, "output.ndjson");

    // Compress with array-headers wrapper
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("array_headers_mixed_types.json")
                .to_str()
                .unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-array-headers",
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

    // Verify types are preserved
    assert_eq!(records[0]["id"].as_i64().unwrap(), 1);
    assert_eq!(records[0]["name"].as_str().unwrap(), "Alice");
    assert_eq!(records[0]["active"].as_bool().unwrap(), true);
    assert_eq!(records[0]["score"].as_f64().unwrap(), 95.5);

    // Verify null handling
    assert_eq!(records[1]["id"].as_i64().unwrap(), 2);
    assert!(records[1]["score"].is_null());

    Ok(())
}

#[test]
fn array_headers_rejects_invalid_header() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let jac_path = temp_output_path(&dir, "output.jac");

    // Attempt to compress with invalid header (non-string element)
    let result = assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("array_headers_invalid_header.json")
                .to_str()
                .unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-array-headers",
        ])
        .assert()
        .failure();

    // Verify error message mentions header validation
    result.stderr(predicate::str::contains("Header element"));

    Ok(())
}

#[test]
fn array_headers_rejects_length_mismatch() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let jac_path = temp_output_path(&dir, "output.jac");

    // Attempt to compress with mismatched row lengths
    let result = assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("array_headers_length_mismatch.json")
                .to_str()
                .unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-array-headers",
        ])
        .assert()
        .failure();

    // Verify error message mentions row length mismatch
    result.stderr(predicate::str::contains("Row length mismatch"));

    Ok(())
}

#[test]
fn array_headers_conflicts_with_other_wrappers() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let jac_path = temp_output_path(&dir, "output.jac");

    // Test conflict with pointer
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("array_headers_basic.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-array-headers",
            "--wrapper-pointer",
            "/data",
        ])
        .assert()
        .failure();

    // Test conflict with sections
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("array_headers_basic.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-array-headers",
            "--wrapper-sections",
            "users",
        ])
        .assert()
        .failure();

    // Test conflict with map
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("array_headers_basic.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-array-headers",
            "--wrapper-map",
        ])
        .assert()
        .failure();

    Ok(())
}

#[test]
fn array_headers_roundtrip() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let jac_path = temp_output_path(&dir, "output.jac");
    let ndjson_path = temp_output_path(&dir, "output.ndjson");
    let jac_path2 = temp_output_path(&dir, "output2.jac");
    let ndjson_path2 = temp_output_path(&dir, "output2.ndjson");

    // First compression with array-headers
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("array_headers_basic.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-array-headers",
        ])
        .assert()
        .success();

    // First decompression
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

    // Second compression (without wrapper, since it's already NDJSON)
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            ndjson_path.to_str().unwrap(),
            "-o",
            jac_path2.to_str().unwrap(),
            "--ndjson",
        ])
        .assert()
        .success();

    // Second decompression
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "unpack",
            jac_path2.to_str().unwrap(),
            "-o",
            ndjson_path2.to_str().unwrap(),
            "--ndjson",
        ])
        .assert()
        .success();

    // Verify both outputs are identical
    let records1 = parse_ndjson(&ndjson_path)?;
    let records2 = parse_ndjson(&ndjson_path2)?;
    assert_eq!(records1, records2);

    Ok(())
}
