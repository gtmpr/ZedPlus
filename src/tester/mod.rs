use anyhow::Result;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub enum TestRunner {
    Cargo,
    Npm,
    Pytest,
    Go,
    None,
}

impl TestRunner {
    pub fn command(&self) -> Option<&'static str> {
        match self {
            TestRunner::Cargo => Some("cargo test"),
            TestRunner::Npm => Some("npm test"),
            TestRunner::Pytest => Some("pytest"),
            TestRunner::Go => Some("go test ./..."),
            TestRunner::None => None,
        }
    }
}

pub fn detect(cwd: &Path) -> TestRunner {
    if cwd.join("Cargo.toml").exists() {
        TestRunner::Cargo
    } else if cwd.join("package.json").exists() {
        TestRunner::Npm
    } else if cwd.join("pytest.ini").exists() || cwd.join("conftest.py").exists() {
        TestRunner::Pytest
    } else if cwd.join("go.mod").exists() {
        TestRunner::Go
    } else {
        TestRunner::None
    }
}

pub mod runner;
