use anyhow::Result;
use std::process::Command;
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::tester::{TestRunner, detect};
use crate::platform::dirs;
use crate::db;
use chrono::Utc;

pub struct TestResult {
    pub passed: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Run tests in the background and log results to SQLite.
pub async fn run_background_tests(cwd: std::path::PathBuf, triggered_by: String, model_key: Option<String>) -> Result<TestResult> {
    let runner = detect(&cwd);
    let cmd = match runner.command() {
        Some(c) => c,
        None => anyhow::bail!("No test runner detected"),
    };

    println!("\n\x1b[90m[tester] Running background tests: {}...\x1b[0m", cmd);
    
    let output = if cfg!(windows) {
        Command::new("cmd")
            .args(["/C", cmd])
            .current_dir(&cwd)
            .output()?
    } else {
        Command::new("sh")
            .args(["-c", cmd])
            .current_dir(&cwd)
            .output()?
    };

    let passed = output.status.success();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    // Log to DB
    let db_path = dirs::db_file()?;
    let conn = db::open(&db_path)?;
    
    conn.execute(
        "INSERT INTO test_runs (ts, runner, triggered_by, model_key, passed, failed, output) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            Utc::now().timestamp(),
            format!("{:?}", runner),
            triggered_by,
            model_key,
            passed as i32,
            (!passed) as i32,
            if passed { stdout.clone() } else { stderr.clone() }
        ],
    )?;

    if !passed {
        eprintln!("\n\x1b[31m[❌ Tests Failed] (run /fix to auto-resolve)\x1b[0m");
    } else {
        println!("\n\x1b[32m[✅ Tests Passed]\x1b[0m");
    }

    Ok(TestResult { passed, stdout, stderr })
}
