use predicates::prelude::*;
use serde_json::Value;
use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use tempfile::TempDir;

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
