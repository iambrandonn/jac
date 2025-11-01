//! Integration tests for wrapper sections mode

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn sections_basic_concatenation() {
    let temp = TempDir::new().unwrap();
    let input = "tests/fixtures/wrapper/sections_basic.json";
    let output = temp.path().join("output.jac");

    // Compress with sections wrapper
    Command::cargo_bin("jac")
        .unwrap()
        .arg("pack")
        .arg(input)
        .arg("-o")
        .arg(&output)
        .arg("--wrapper-sections")
        .arg("users")
        .arg("guests")
        .assert()
        .success();

    assert!(output.exists());

    // Decompress and verify
    let decompressed = temp.path().join("output.ndjson");
    Command::cargo_bin("jac")
        .unwrap()
        .arg("unpack")
        .arg(&output)
        .arg("-o")
        .arg(&decompressed)
        .arg("--ndjson")
        .assert()
        .success();

    let content = fs::read_to_string(&decompressed).unwrap();
    let lines: Vec<&str> = content.trim().lines().collect();

    // Should have 3 records total (2 users + 1 guest)
    assert_eq!(lines.len(), 3);

    // Verify content
    assert!(lines[0].contains("alice"));
    assert!(lines[1].contains("bob"));
    assert!(lines[2].contains("charlie"));
}

#[test]
fn sections_with_label_injection() {
    let temp = TempDir::new().unwrap();
    let input = "tests/fixtures/wrapper/sections_basic.json";
    let output = temp.path().join("output.jac");

    // Compress with sections wrapper and label injection
    Command::cargo_bin("jac")
        .unwrap()
        .arg("pack")
        .arg(input)
        .arg("-o")
        .arg(&output)
        .arg("--wrapper-sections")
        .arg("users")
        .arg("guests")
        .assert()
        .success();

    // Decompress and verify labels
    let decompressed = temp.path().join("output.ndjson");
    Command::cargo_bin("jac")
        .unwrap()
        .arg("unpack")
        .arg(&output)
        .arg("-o")
        .arg(&decompressed)
        .arg("--ndjson")
        .assert()
        .success();

    let content = fs::read_to_string(&decompressed).unwrap();

    // By default, labels should be injected
    assert!(content.contains("_section"));
    assert!(content.contains("users"));
    assert!(content.contains("guests"));
}

#[test]
fn sections_with_custom_label_field() {
    let temp = TempDir::new().unwrap();
    let input = "tests/fixtures/wrapper/sections_basic.json";
    let output = temp.path().join("output.jac");

    // Compress with custom label field
    Command::cargo_bin("jac")
        .unwrap()
        .arg("pack")
        .arg(input)
        .arg("-o")
        .arg(&output)
        .arg("--wrapper-sections")
        .arg("users")
        .arg("guests")
        .arg("--wrapper-section-label-field")
        .arg("source")
        .assert()
        .success();

    let decompressed = temp.path().join("output.ndjson");
    Command::cargo_bin("jac")
        .unwrap()
        .arg("unpack")
        .arg(&output)
        .arg("-o")
        .arg(&decompressed)
        .arg("--ndjson")
        .assert()
        .success();

    let content = fs::read_to_string(&decompressed).unwrap();

    // Should use custom field name
    assert!(content.contains("\"source\""));
}

#[test]
fn sections_no_label() {
    let temp = TempDir::new().unwrap();
    let input = "tests/fixtures/wrapper/sections_basic.json";
    let output = temp.path().join("output.jac");

    // Compress without label injection
    Command::cargo_bin("jac")
        .unwrap()
        .arg("pack")
        .arg(input)
        .arg("-o")
        .arg(&output)
        .arg("--wrapper-sections")
        .arg("users")
        .arg("guests")
        .arg("--wrapper-section-no-label")
        .assert()
        .success();

    let decompressed = temp.path().join("output.ndjson");
    Command::cargo_bin("jac")
        .unwrap()
        .arg("unpack")
        .arg(&output)
        .arg("-o")
        .arg(&decompressed)
        .arg("--ndjson")
        .assert()
        .success();

    let content = fs::read_to_string(&decompressed).unwrap();

    // Should not have section labels
    assert!(!content.contains("_section"));
}

#[test]
fn sections_custom_pointer() {
    let temp = TempDir::new().unwrap();

    // Create nested structure
    let input_file = temp.path().join("nested.json");
    fs::write(&input_file, r#"{
  "data": {
    "users": [
      {"name": "alice"}
    ],
    "admins": [
      {"name": "bob"}
    ]
  }
}"#).unwrap();

    let output = temp.path().join("output.jac");

    // Compress with custom pointers
    Command::cargo_bin("jac")
        .unwrap()
        .arg("pack")
        .arg(&input_file)
        .arg("-o")
        .arg(&output)
        .arg("--wrapper-sections")
        .arg("users")
        .arg("admins")
        .arg("--wrapper-section-pointer")
        .arg("users=/data/users")
        .arg("--wrapper-section-pointer")
        .arg("admins=/data/admins")
        .assert()
        .success();

    let decompressed = temp.path().join("output.ndjson");
    Command::cargo_bin("jac")
        .unwrap()
        .arg("unpack")
        .arg(&output)
        .arg("-o")
        .arg(&decompressed)
        .arg("--ndjson")
        .assert()
        .success();

    let content = fs::read_to_string(&decompressed).unwrap();
    let lines: Vec<&str> = content.trim().lines().collect();

    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("alice"));
    assert!(lines[1].contains("bob"));
}

#[test]
fn sections_missing_skip() {
    let temp = TempDir::new().unwrap();
    let input = "tests/fixtures/wrapper/sections_basic.json";
    let output = temp.path().join("output.jac");

    // Request a missing section (should skip by default)
    Command::cargo_bin("jac")
        .unwrap()
        .arg("pack")
        .arg(input)
        .arg("-o")
        .arg(&output)
        .arg("--wrapper-sections")
        .arg("users")
        .arg("missing")
        .arg("guests")
        .assert()
        .success();

    let decompressed = temp.path().join("output.ndjson");
    Command::cargo_bin("jac")
        .unwrap()
        .arg("unpack")
        .arg(&output)
        .arg("-o")
        .arg(&decompressed)
        .arg("--ndjson")
        .assert()
        .success();

    let content = fs::read_to_string(&decompressed).unwrap();
    let lines: Vec<&str> = content.trim().lines().collect();

    // Should only have users and guests (missing skipped)
    assert_eq!(lines.len(), 3);
}

#[test]
fn sections_missing_error() {
    let temp = TempDir::new().unwrap();
    let input = "tests/fixtures/wrapper/sections_basic.json";
    let output = temp.path().join("output.jac");

    // Request a missing section with error mode
    Command::cargo_bin("jac")
        .unwrap()
        .arg("pack")
        .arg(input)
        .arg("-o")
        .arg(&output)
        .arg("--wrapper-sections")
        .arg("missing")
        .arg("--wrapper-sections-missing-error")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Section not found"));
}

#[test]
fn sections_conflicts_with_pointer() {
    let temp = TempDir::new().unwrap();
    let input = "tests/fixtures/wrapper/sections_basic.json";
    let output = temp.path().join("output.jac");

    // Try to use both wrapper modes at once
    Command::cargo_bin("jac")
        .unwrap()
        .arg("pack")
        .arg(input)
        .arg("-o")
        .arg(&output)
        .arg("--wrapper-pointer")
        .arg("/data")
        .arg("--wrapper-sections")
        .arg("users")
        .assert()
        .failure();
}

#[test]
fn sections_requires_base_flag() {
    let temp = TempDir::new().unwrap();
    let input = "tests/fixtures/wrapper/sections_basic.json";
    let output = temp.path().join("output.jac");

    // Try to use section flags without --wrapper-sections
    Command::cargo_bin("jac")
        .unwrap()
        .arg("pack")
        .arg(input)
        .arg("-o")
        .arg(&output)
        .arg("--wrapper-section-label-field")
        .arg("source")
        .assert()
        .failure();
}
