#!/usr/bin/env pwsh
# ZedPlus install script — run from the project root.
# Usage:
#   .\install.ps1              # build + install current code, no version bump
#   .\install.ps1 patch        # bump patch version (0.1.x → 0.1.x+1), build, install
#   .\install.ps1 minor        # bump minor version (0.x.0), build, install
#   .\install.ps1 major        # bump major version (x.0.0), build, install

param(
    [string]$Bump = ""
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

# ── Toolchain paths ──────────────────────────────────────────────────────────
$mingw   = "C:\msys64\mingw64\bin"
$cargoBin = "$env:USERPROFILE\.cargo\bin"

foreach ($p in @($mingw, $cargoBin)) {
    if ((Test-Path $p) -and ($env:PATH -notlike "*$p*")) {
        $env:PATH = "$p;$env:PATH"
    }
}

# ── Permanently add ~/.cargo/bin to user PATH (once) ────────────────────────
$userPath = [Environment]::GetEnvironmentVariable("PATH", "User") ?? ""
if ($userPath -notlike "*\.cargo\bin*") {
    [Environment]::SetEnvironmentVariable("PATH", "$userPath;$cargoBin", "User")
    Write-Host "Added $cargoBin to your permanent PATH. Restart any open terminals."
}

# ── Read current version ──────────────────────────────────────────────────────
$cargoToml = Get-Content "Cargo.toml" -Raw
if ($cargoToml -notmatch 'version\s*=\s*"(\d+)\.(\d+)\.(\d+)"') {
    Write-Error "Could not parse version from Cargo.toml"
    exit 1
}
$major = [int]$Matches[1]
$minor = [int]$Matches[2]
$patch = [int]$Matches[3]
$oldVersion = "$major.$minor.$patch"

# ── Bump version if requested ─────────────────────────────────────────────────
$newVersion = $oldVersion
switch ($Bump.ToLower()) {
    "patch" { $patch++; $newVersion = "$major.$minor.$patch" }
    "minor" { $minor++; $patch = 0; $newVersion = "$major.$minor.$patch" }
    "major" { $major++; $minor = 0; $patch = 0; $newVersion = "$major.$minor.$patch" }
    ""      { }
    default { Write-Error "Unknown bump type '$Bump'. Use: patch, minor, major"; exit 1 }
}

if ($newVersion -ne $oldVersion) {
    $cargoToml = $cargoToml -replace "version\s*=\s*`"$oldVersion`"", "version = `"$newVersion`""
    Set-Content "Cargo.toml" $cargoToml -NoNewline
    Write-Host "Version: $oldVersion -> $newVersion"

    # Prepend changelog entry
    $date = (Get-Date).ToString("yyyy-MM-dd")
    $entry = "## [$newVersion] - $date`n`n### Changed`n- (add notes here)`n`n---`n`n"
    $changelog = Get-Content "CHANGELOG.md" -Raw
    $split = $changelog -replace "^(# Changelog.*?---\s*\n)", "`$1`n$entry"
    Set-Content "CHANGELOG.md" $split -NoNewline
    Write-Host "Prepended entry to CHANGELOG.md — fill in notes before committing."
}

# ── Build ────────────────────────────────────────────────────────────────────
Write-Host ""
Write-Host "Building zedplus $newVersion..."
cargo build --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# ── Install ──────────────────────────────────────────────────────────────────
$src  = "target\x86_64-pc-windows-gnu\release\zedplus.exe"
$dest = "$cargoBin\zedplus.exe"

if (-not (Test-Path $src)) {
    Write-Error "Build output not found at: $src"
    exit 1
}

# Release the lock if zedplus is currently running
Stop-Process -Name "zedplus" -Force -ErrorAction SilentlyContinue

Copy-Item $src $dest -Force
Write-Host ""
Write-Host "Installed -> $dest"
& $dest --version

# ── Refresh PATH in this session so the new binary is found immediately ───────
$machinePath = [Environment]::GetEnvironmentVariable("PATH", "Machine") ?? ""
$userPath    = [Environment]::GetEnvironmentVariable("PATH", "User")    ?? ""
$env:PATH    = "$machinePath;$userPath"
Write-Host "PATH refreshed — zedplus $newVersion is live in this terminal."
