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
fn basic_map_flattening() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let jac_path = temp_output_path(&dir, "output.jac");
    let ndjson_path = temp_output_path(&dir, "output.ndjson");

    // Compress with map wrapper
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("map_basic.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-map",
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

    // Check that all records have the _key field
    let keys: Vec<String> = records
        .iter()
        .map(|r| r["_key"].as_str().unwrap().to_string())
        .collect();
    assert!(keys.contains(&"alice".to_string()));
    assert!(keys.contains(&"bob".to_string()));
    assert!(keys.contains(&"carol".to_string()));

    // Verify original fields are preserved
    for record in &records {
        assert!(record.get("age").is_some());
        assert!(record.get("role").is_some());
        assert!(record.get("active").is_some());
    }

    Ok(())
}

#[test]
fn map_with_custom_key_field() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let jac_path = temp_output_path(&dir, "output.jac");
    let ndjson_path = temp_output_path(&dir, "output.ndjson");

    // Compress with custom key field
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("map_basic.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-map",
            "--wrapper-map-key-field",
            "user_id",
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

    // Check that all records have user_id instead of _key
    for record in &records {
        assert!(record.get("user_id").is_some());
        assert!(record.get("_key").is_none());
    }

    Ok(())
}

#[test]
fn map_with_nested_pointer() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let jac_path = temp_output_path(&dir, "output.jac");
    let ndjson_path = temp_output_path(&dir, "output.ndjson");

    // Compress with nested pointer
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("map_nested_pointer.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-map",
            "--wrapper-map-pointer",
            "/data/users",
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

    let keys: Vec<String> = records
        .iter()
        .map(|r| r["_key"].as_str().unwrap().to_string())
        .collect();
    assert!(keys.contains(&"user_001".to_string()));
    assert!(keys.contains(&"user_002".to_string()));

    Ok(())
}

#[test]
fn map_empty() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let jac_path = temp_output_path(&dir, "output.jac");
    let ndjson_path = temp_output_path(&dir, "output.ndjson");

    // Compress empty map
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("map_empty.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-map",
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
fn map_collision_error_mode() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let jac_path = temp_output_path(&dir, "output.jac");

    // Compress with collision (should fail)
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("map_collision.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-map",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Key field collision"));

    Ok(())
}

#[test]
fn map_collision_overwrite_mode() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let jac_path = temp_output_path(&dir, "output.jac");
    let ndjson_path = temp_output_path(&dir, "output.ndjson");

    // Compress with overwrite mode
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("map_collision.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-map",
            "--wrapper-map-overwrite-key",
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

    // Verify the key was overwritten with the map key
    let records = parse_ndjson(&ndjson_path)?;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["_key"], "alice");

    Ok(())
}

#[test]
fn map_flag_conflicts() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let jac_path = temp_output_path(&dir, "output.jac");

    // --wrapper-map conflicts with --wrapper-pointer
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("map_basic.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-map",
            "--wrapper-pointer",
            "/data",
        ])
        .assert()
        .failure();

    // --wrapper-map conflicts with --wrapper-sections
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("map_basic.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-map",
            "--wrapper-sections",
            "users",
        ])
        .assert()
        .failure();

    Ok(())
}

#[test]
fn map_flag_dependencies() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let jac_path = temp_output_path(&dir, "output.jac");

    // --wrapper-map-pointer requires --wrapper-map
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("map_basic.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-map-pointer",
            "/data",
        ])
        .assert()
        .failure();

    // --wrapper-map-key-field requires --wrapper-map
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("map_basic.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-map-key-field",
            "id",
        ])
        .assert()
        .failure();

    // --wrapper-map-overwrite-key requires --wrapper-map
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture_path("map_basic.json").to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--wrapper-map-overwrite-key",
        ])
        .assert()
        .failure();

    Ok(())
}
