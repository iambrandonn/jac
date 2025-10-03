use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    match run() {
        Ok(()) => {}
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }
}

fn run() -> Result<(), String> {
    let repo = repo_root()?;
    let matrix_path = repo.join("docs/compliance_matrix.csv");
    if !matrix_path.exists() {
        return Err(format!(
            "compliance matrix missing at {}",
            matrix_path.display()
        ));
    }

    let content = fs::read_to_string(&matrix_path)
        .map_err(|e| format!("reading {} failed: {e}", matrix_path.display()))?;

    let mut missing_tests: Vec<String> = Vec::new();
    let mut missing_files: Vec<(String, PathBuf)> = Vec::new();

    for (idx, line) in content.lines().enumerate() {
        if idx == 0 {
            continue; // header
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let columns: Vec<&str> = line.split(',').collect();
        if columns.len() < 5 {
            return Err(format!(
                "line {}: expected 5 columns, found {}",
                idx + 1,
                columns.len()
            ));
        }

        let requirement = columns[0].trim();
        if requirement.is_empty() {
            return Err(format!("line {}: requirement_id empty", idx + 1));
        }

        let test_refs = columns[4].trim();
        if test_refs.is_empty() {
            missing_tests.push(format!("{} (line {})", requirement, idx + 1));
            continue;
        }

        for reference in test_refs.split(';') {
            let reference = reference.trim();
            if reference.is_empty() {
                continue;
            }
            let path_part = reference.split("::").next().unwrap_or(reference).trim();
            if path_part.is_empty() {
                continue;
            }
            let path = repo.join(path_part);
            if !path.exists() {
                missing_files.push((
                    format!("{} (line {}) ref {}", requirement, idx + 1, reference),
                    path,
                ));
            }
        }
    }

    if missing_tests.is_empty() && missing_files.is_empty() {
        println!("Compliance matrix OK");
        return Ok(());
    }

    let mut message = String::from("Compliance matrix validation failed:\n");
    if !missing_tests.is_empty() {
        message.push_str("Rows with empty test_refs column:\n");
        for row in &missing_tests {
            message.push_str("  - ");
            message.push_str(row);
            message.push('\n');
        }
    }
    if !missing_files.is_empty() {
        message.push_str("Referenced files not found:\n");
        for (row, file) in &missing_files {
            message.push_str(&format!("  - {} -> {}\n", row, file.display()));
        }
    }

    Err(message)
}

fn repo_root() -> Result<PathBuf, String> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest_dir)
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "xtask manifest directory has no parent".to_string())
}
