use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let args: Vec<String> = env::args().collect();
    let command = args.get(1).map(|s| s.as_str()).unwrap_or("validate");

    match command {
        "validate" => run_validate(),
        "test" => run_test(),
        "format" => run_format(),
        "clippy" => run_clippy(),
        "conformance" => run_conformance(),
        "fuzz" => run_fuzz(),
        "all" => run_all(),
        "help" => print_help(),
        _ => {
            eprintln!("Unknown command: {}", command);
            print_help();
            std::process::exit(1);
        }
    }
}

fn print_help() {
    println!("JAC Validation Tool");
    println!();
    println!("Usage: cargo run -p xtask [COMMAND]");
    println!();
    println!("Commands:");
    println!("  validate     - Validate compliance matrix (default)");
    println!("  test         - Run all tests");
    println!("  format       - Format code with rustfmt");
    println!("  clippy       - Run clippy linter");
    println!("  conformance  - Run conformance test suite with detailed reporting");
    println!("  fuzz         - Run fuzzing targets");
    println!("  all          - Run all validation steps");
    println!("  help         - Show this help message");
}

fn run_validate() {
    match validate_compliance_matrix() {
        Ok(()) => println!("✅ Compliance matrix validation passed"),
        Err(err) => {
            eprintln!("❌ Compliance matrix validation failed: {}", err);
            std::process::exit(1);
        }
    }
}

fn run_test() {
    println!("🧪 Running all tests...");
    let status = Command::new("cargo")
        .args(&["test", "--all"])
        .status()
        .expect("Failed to run cargo test");

    if !status.success() {
        eprintln!("❌ Tests failed");
        std::process::exit(1);
    }
    println!("✅ All tests passed");
}

fn run_format() {
    println!("🎨 Formatting code...");
    let status = Command::new("cargo")
        .args(&["fmt", "--all"])
        .status()
        .expect("Failed to run cargo fmt");

    if !status.success() {
        eprintln!("❌ Formatting failed");
        std::process::exit(1);
    }
    println!("✅ Code formatted");
}

fn run_clippy() {
    println!("🔍 Running clippy...");
    let status = Command::new("cargo")
        .args(&["clippy", "--all", "--", "-D", "warnings"])
        .status()
        .expect("Failed to run cargo clippy");

    if !status.success() {
        eprintln!("❌ Clippy found issues");
        std::process::exit(1);
    }
    println!("✅ Clippy passed");
}

fn run_conformance() {
    println!("🧪 Running conformance test suite...");

    // Run conformance tests with detailed output
    let status = Command::new("cargo")
        .args(&["test", "-p", "jac-codec", "--test", "conformance", "--", "--nocapture"])
        .status()
        .expect("Failed to run conformance tests");

    if !status.success() {
        eprintln!("❌ Conformance tests failed");
        std::process::exit(1);
    }

    println!("✅ Conformance tests passed");

    // Generate conformance report
    generate_conformance_report();
}

fn run_fuzz() {
    println!("🔬 Running fuzzing targets...");

    // Check if cargo-fuzz is installed
    let fuzz_check = Command::new("cargo")
        .args(&["fuzz", "--help"])
        .output();

    if fuzz_check.is_err() {
        eprintln!("❌ cargo-fuzz not installed. Install with: cargo install cargo-fuzz");
        std::process::exit(1);
    }

    // Run fuzz targets for a short time
    let fuzz_targets = ["fuzz_varint", "fuzz_decode_block", "fuzz_projection", "fuzz_compression", "fuzz_bitpack"];

    for target in &fuzz_targets {
        println!("🔍 Running fuzz target: {}", target);
        let status = Command::new("cargo")
            .args(&["fuzz", "run", target, "--", "-max_total_time=10"])
            .current_dir("jac-codec")
            .status()
            .expect("Failed to run fuzz target");

        if !status.success() {
            eprintln!("❌ Fuzz target {} failed", target);
            std::process::exit(1);
        }
    }

    println!("✅ All fuzz targets completed");
}

fn run_all() {
    println!("🚀 Running all validation steps...");

    run_format();
    run_clippy();
    run_test();
    run_conformance();
    run_validate();

    println!("🎉 All validation steps completed successfully!");
}

fn validate_compliance_matrix() -> Result<(), String> {
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

        let columns = parse_csv_line(line);
        if columns.len() < 5 {
            eprintln!("Debug: line {}: '{}'", idx + 1, line);
            eprintln!("Debug: parsed columns: {:?}", columns);
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

fn parse_csv_line(line: &str) -> Vec<String> {
    let mut columns = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
            }
            ',' if !in_quotes => {
                columns.push(current.trim().to_string());
                current.clear();
            }
            _ => {
                current.push(ch);
            }
        }
    }

    // Add the last column
    columns.push(current.trim().to_string());
    columns
}

fn generate_conformance_report() {
    let repo = match repo_root() {
        Ok(path) => path,
        Err(e) => {
            eprintln!("❌ Failed to get repo root: {}", e);
            return;
        }
    };

    let report_path = repo.join("conformance_report.md");
    let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");

    let report = format!(
        "# JAC Conformance Test Report\n\n\
        Generated: {}\n\n\
        ## Test Coverage\n\n\
        - ✅ SPEC §12.1 conformance tests\n\
        - ✅ Schema drift validation\n\
        - ✅ Multi-level validation (columnar/segment inspection)\n\
        - ✅ Edge case testing (deeply nested, high-precision decimals, Unicode)\n\
        - ✅ Boundary value testing\n\
        - ✅ Large synthetic block testing\n\n\
        ## Test Results\n\n\
        All conformance tests passed successfully.\n\n\
        ## Compliance Status\n\n\
        See `docs/compliance_matrix.csv` for detailed compliance tracking.\n",
        timestamp
    );

    if let Err(e) = fs::write(&report_path, report) {
        eprintln!("❌ Failed to write conformance report: {}", e);
    } else {
        println!("📊 Conformance report written to: {}", report_path.display());
    }
}

fn repo_root() -> Result<PathBuf, String> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest_dir)
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "xtask manifest directory has no parent".to_string())
}
