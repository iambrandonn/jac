use jac_format::constants::{ENCODING_FLAG_DELTA, ENCODING_FLAG_DICTIONARY};
use predicates::prelude::*;
use serde_json::Value;
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};

struct SampleFile {
    _dir: TempDir,
    jac_path: PathBuf,
}

fn build_sample_file() -> Result<SampleFile, Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let input_path = dir.path().join("input.ndjson");
    let jac_path = dir.path().join("output.jac");

    let mut file = fs::File::create(&input_path)?;
    file.write_all(b"{\"user\":\"alice\",\"level\":\"info\"}\n")?;
    file.write_all(b"{\"user\":\"bob\",\"level\":\"warn\"}\n")?;

    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            input_path.to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    Ok(SampleFile {
        _dir: dir,
        jac_path,
    })
}

fn spec_fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../testdata/spec/v12_1.jsonl")
}

fn load_fixture_values(path: &Path) -> Result<Vec<Value>, Box<dyn Error>> {
    let contents = fs::read_to_string(path)?;
    Ok(contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect())
}

#[test]
fn ls_table_lists_fields() -> Result<(), Box<dyn Error>> {
    let sample = build_sample_file()?;
    let output = assert_cmd::Command::cargo_bin("jac")?
        .args(["ls", sample.jac_path.to_str().unwrap()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output)?;
    assert!(stdout.contains("user"));
    assert!(stdout.contains("level"));
    Ok(())
}

#[test]
fn ls_json_output_parses() -> Result<(), Box<dyn Error>> {
    let sample = build_sample_file()?;
    let output = assert_cmd::Command::cargo_bin("jac")?
        .args(["ls", sample.jac_path.to_str().unwrap(), "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output)?;
    let fields = value["fields"].as_array().unwrap();
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0], "level");
    assert_eq!(fields[1], "user");
    Ok(())
}

#[test]
fn ls_stats_reports_field_details() -> Result<(), Box<dyn Error>> {
    let sample = build_sample_file()?;
    let output = assert_cmd::Command::cargo_bin("jac")?
        .args([
            "ls",
            sample.jac_path.to_str().unwrap(),
            "--format",
            "json",
            "--stats",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output)?;
    assert_eq!(value["stats_sample_limit"], Value::from(50_000));
    let stats = value["stats"].as_array().expect("stats array");
    assert_eq!(stats.len(), 2);
    let user_entry = stats
        .iter()
        .find(|entry| entry["field_name"] == "user")
        .expect("user stats present");
    assert_eq!(user_entry["present_values"], 2);
    assert_eq!(user_entry["null_count"], 0);
    assert_eq!(user_entry["absent_values"], 0);
    let types = user_entry["type_distribution"].as_object().unwrap();
    assert_eq!(types.get("string").cloned().unwrap(), Value::from(2));
    assert_eq!(user_entry["sampled"], Value::Bool(false));
    assert_eq!(user_entry["sample_size"], Value::from(2));
    Ok(())
}

#[test]
fn ls_stats_respects_sample_limit_flag() -> Result<(), Box<dyn Error>> {
    let sample = build_sample_file()?;
    let output = assert_cmd::Command::cargo_bin("jac")?
        .args([
            "ls",
            sample.jac_path.to_str().unwrap(),
            "--format",
            "json",
            "--stats",
            "--stats-sample",
            "1",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output)?;
    assert_eq!(value["stats_sample_limit"], Value::from(1));
    let stats = value["stats"].as_array().unwrap();
    let user_entry = stats
        .iter()
        .find(|entry| entry["field_name"] == "user")
        .unwrap();
    assert_eq!(user_entry["sampled"], Value::Bool(true));
    assert_eq!(user_entry["sample_size"], Value::from(1));
    Ok(())
}

#[test]
fn cat_ndjson_outputs_values() -> Result<(), Box<dyn Error>> {
    let sample = build_sample_file()?;
    let output = assert_cmd::Command::cargo_bin("jac")?
        .args(["cat", sample.jac_path.to_str().unwrap(), "--field", "user"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output)?;
    let values: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        values,
        vec![Value::String("alice".into()), Value::String("bob".into())]
    );
    Ok(())
}

#[test]
fn cat_csv_outputs_plain_values() -> Result<(), Box<dyn Error>> {
    let sample = build_sample_file()?;
    let output = assert_cmd::Command::cargo_bin("jac")?
        .args([
            "cat",
            sample.jac_path.to_str().unwrap(),
            "--field",
            "level",
            "--format",
            "csv",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output)?;
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines, vec!["info", "warn"]);
    Ok(())
}

#[test]
fn cat_unknown_field_fails() -> Result<(), Box<dyn Error>> {
    let sample = build_sample_file()?;
    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "cat",
            sample.jac_path.to_str().unwrap(),
            "--field",
            "missing",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Available fields: level, user"));
    Ok(())
}

#[test]
fn unpack_auto_defaults_to_ndjson_output() -> Result<(), Box<dyn Error>> {
    let sample = build_sample_file()?;
    let dir = tempdir()?;
    let output_path = dir.path().join("out.ndjson");

    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "unpack",
            sample.jac_path.to_str().unwrap(),
            "-o",
            output_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    let contents = fs::read_to_string(&output_path)?;
    let lines: Vec<_> = contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    assert_eq!(lines.len(), 2);
    for line in &lines {
        serde_json::from_str::<Value>(line)?;
    }
    assert!(!contents.trim_start().starts_with('['));
    Ok(())
}

#[test]
fn unpack_auto_preserves_json_array_wrapper() -> Result<(), Box<dyn Error>> {
    let dir = tempdir()?;
    let input_path = dir.path().join("input.json");
    let jac_path = dir.path().join("output.jac");
    let output_path = dir.path().join("decoded.json");

    fs::write(&input_path, r#"[{"id":1},{"id":2}]"#)?;

    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            input_path.to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "unpack",
            jac_path.to_str().unwrap(),
            "-o",
            output_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    let decoded = fs::read_to_string(&output_path)?;
    let value: Value = serde_json::from_str(&decoded)?;
    assert!(value.is_array());
    assert_eq!(value.as_array().unwrap().len(), 2);
    Ok(())
}

#[test]
fn pack_accepts_bom_prefixed_ndjson() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let input_path = dir.path().join("bom.ndjson");
    let jac_path = dir.path().join("bom.jac");

    let mut file = fs::File::create(&input_path)?;
    file.write_all("\u{feff}{\"value\":1}\r\n".as_bytes())?;
    file.write_all("{\"value\":2}\n".as_bytes())?;
    file.write_all("{\"value\":3}".as_bytes())?;

    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            input_path.to_str().unwrap(),
            "-o",
            jac_path.to_str().unwrap(),
            "--ndjson",
        ])
        .assert()
        .success();

    let output = assert_cmd::Command::cargo_bin("jac")?
        .args(["cat", jac_path.to_str().unwrap(), "--field", "value"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let values: Vec<i64> = String::from_utf8(output)?
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .map(|value| value.as_i64().unwrap())
        .collect();
    assert_eq!(values, vec![1, 2, 3]);

    Ok(())
}

#[test]
fn pack_requires_allow_flag_for_large_segments() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let input_path = dir.path().join("input.ndjson");
    let output_path = dir.path().join("output.jac");
    fs::write(&input_path, "{\"value\":1}\n")?;

    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            input_path.to_str().unwrap(),
            "-o",
            output_path.to_str().unwrap(),
            "--max-segment-bytes",
            "134217728",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--allow-large-segments"));
    Ok(())
}

#[test]
fn pack_allows_large_segments_with_confirmation() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let input_path = dir.path().join("input.ndjson");
    let output_path = dir.path().join("output.jac");
    fs::write(&input_path, "{\"value\":1}\n{\"value\":2}\n")?;

    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            input_path.to_str().unwrap(),
            "-o",
            output_path.to_str().unwrap(),
            "--max-segment-bytes",
            "134217728",
            "--allow-large-segments",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "Warning: increasing --max-segment-bytes",
        ));
    Ok(())
}

#[test]
fn spec_fixture_cli_conformance() -> Result<(), Box<dyn Error>> {
    let fixture = spec_fixture_path();
    let expected_records = load_fixture_values(&fixture)?;
    let expected_users: Vec<String> = expected_records
        .iter()
        .map(|record| record["user"].as_str().expect("user present").to_owned())
        .collect();

    let dir = tempfile::tempdir()?;
    let packed = dir.path().join("spec.jac");
    let unpacked = dir.path().join("spec.ndjson");

    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            fixture.to_str().unwrap(),
            "-o",
            packed.to_str().unwrap(),
            "--ndjson",
        ])
        .assert()
        .success();

    let cat_output = assert_cmd::Command::cargo_bin("jac")?
        .args(["cat", packed.to_str().unwrap(), "--field", "user"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let cat_users: Vec<String> = String::from_utf8(cat_output)?
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .map(|value| value.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(cat_users, expected_users, "projection matches SPEC §12.1");

    assert_cmd::Command::cargo_bin("jac")?
        .args([
            "unpack",
            packed.to_str().unwrap(),
            "-o",
            unpacked.to_str().unwrap(),
            "--ndjson",
        ])
        .assert()
        .success();
    let decompressed = load_fixture_values(&unpacked)?;
    assert_eq!(decompressed, expected_records, "round-trip NDJSON matches");

    let ls_output = assert_cmd::Command::cargo_bin("jac")?
        .args([
            "ls",
            packed.to_str().unwrap(),
            "--format",
            "json",
            "--stats",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let ls_json: Value = serde_json::from_slice(&ls_output)?;
    let fields_array = ls_json["blocks"][0]["fields"]
        .as_array()
        .expect("fields array present");
    let mut field_meta: HashMap<&str, &Value> = HashMap::new();
    for field in fields_array {
        let name = field["name"].as_str().unwrap();
        field_meta.insert(name, field);
    }

    let ts_flags = field_meta["ts"]["encoding_flags"].as_u64().unwrap();
    assert_ne!(ts_flags & ENCODING_FLAG_DELTA, 0, "ts has delta flag");

    let level_flags = field_meta["level"]["encoding_flags"].as_u64().unwrap();
    assert_ne!(
        level_flags & ENCODING_FLAG_DICTIONARY,
        0,
        "level uses dictionary"
    );

    let error_field = field_meta["error"];
    let error_flags = error_field["encoding_flags"].as_u64().unwrap();
    assert_ne!(
        error_flags & ENCODING_FLAG_DICTIONARY,
        0,
        "error dictionary due to single value"
    );
    assert_eq!(
        error_field["present_count"].as_u64().unwrap(),
        1,
        "error present once"
    );

    Ok(())
}

#[test]
fn segment_limit_error_message_is_helpful() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let input_path = dir.path().join("input.ndjson");
    let output_path = dir.path().join("output.jac");

    // Create a record with a large field (1 MiB string)
    let large_value = "x".repeat(1024 * 1024);
    let record = serde_json::json!({"large_field": large_value});
    fs::write(&input_path, format!("{}\n", record.to_string()))?;

    // Try to pack with a very small segment limit (will fail)
    let result = assert_cmd::Command::cargo_bin("jac")?
        .args([
            "pack",
            input_path.to_str().unwrap(),
            "-o",
            output_path.to_str().unwrap(),
            "--max-segment-bytes",
            "512000", // 500 KiB - too small for 1 MiB field
            "--allow-large-segments",
        ])
        .assert()
        .failure();

    // Verify the error message contains helpful information
    let output = result.get_output();
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should mention the field name
    assert!(
        stderr.contains("large_field"),
        "Error should mention field name 'large_field', got: {}",
        stderr
    );

    // Should mention that it's a single-record payload issue
    assert!(
        stderr.contains("single-record payload"),
        "Error should mention 'single-record payload', got: {}",
        stderr
    );

    // Should provide recommendation with --max-segment-bytes
    assert!(
        stderr.contains("--max-segment-bytes"),
        "Error should suggest --max-segment-bytes, got: {}",
        stderr
    );

    // Should warn about risks
    assert!(
        stderr.contains("WARNING") || stderr.contains("Warning"),
        "Error should contain warning about risks, got: {}",
        stderr
    );

    Ok(())
}
