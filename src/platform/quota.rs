use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::dirs;
use crate::router::classifier::TaskType;

/// Per-provider quota state persisted between sessions.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderQuota {
    /// Tokens remaining in the current rate-limit window (from API response headers).
    pub tokens_remaining: Option<u64>,
    /// Total token limit for the window (from API response headers).
    pub tokens_limit: Option<u64>,
    /// Unix timestamp when the current window resets (parsed from ISO-8601 header).
    pub reset_at: Option<i64>,
    /// True when the CLI subscription hit its usage cap.
    pub cli_capped: bool,
    /// Unix timestamp when we expect the CLI cap to clear (heuristic: +4 h for Claude).
    pub cli_cap_reset_at: Option<i64>,
    /// Last time this entry was updated (unix timestamp).
    pub updated_at: i64,
}

/// Cache of quota state for all providers, written to ~/.config/zedplus/quota_cache.json.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QuotaCache {
    pub claude: ProviderQuota,
    /// Estimated tokens used today against the Gemini API/CLI (summed from `usage` DB).
    pub gemini_tokens_today: u64,
    /// Daily token budget for Gemini (from config `[quotas] gemini_daily_tokens`).
    pub gemini_daily_budget: u64,
}

impl QuotaCache {
    /// Load from disk; silently return default if the file is missing or corrupt.
    pub fn load() -> Self {
        let path = match dirs::quota_cache_file() {
            Ok(p) => p,
            Err(_) => return Self::default(),
        };
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => return Self::default(),
        };
        serde_json::from_slice(&bytes).unwrap_or_default()
    }

    /// Persist to disk. Silent on error.
    pub fn save(&self) {
        if let Ok(path) = dirs::quota_cache_file() {
            if let Ok(json) = serde_json::to_vec_pretty(self) {
                let _ = std::fs::write(path, json);
            }
        }
    }

    /// Called on every successful Claude API response to harvest ratelimit headers.
    pub fn update_from_claude_headers(
        &mut self,
        remaining: u64,
        limit: u64,
        reset_iso: &str,
    ) {
        self.claude.tokens_remaining = Some(remaining);
        if limit > 0 {
            self.claude.tokens_limit = Some(limit);
        }
        if !reset_iso.is_empty() {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(reset_iso) {
                self.claude.reset_at = Some(dt.timestamp());
            }
        }
        self.claude.updated_at = Utc::now().timestamp();
    }

    /// Called when the Claude CLI returns a rate-limit error.
    pub fn mark_claude_cli_capped(&mut self) {
        self.claude.cli_capped = true;
        self.claude.cli_cap_reset_at = Some(Utc::now().timestamp() + 4 * 3600);
        self.claude.updated_at = Utc::now().timestamp();
        self.save();
    }

    /// Re-estimate today's Gemini usage from the `usage` SQLite table and apply
    /// the configured daily budget.
    pub fn refresh_gemini(&mut self, daily_budget: u64) -> Result<()> {
        self.gemini_daily_budget = daily_budget;
        let db_path = dirs::db_file()?;
        if !db_path.exists() {
            return Ok(());
        }
        let conn = rusqlite::Connection::open(&db_path)?;
        let day_start = Utc::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .map(|dt| dt.and_utc().timestamp())
            .unwrap_or(0);
        let tokens: i64 = conn.query_row(
            "SELECT COALESCE(SUM(input_tokens + output_tokens), 0) \
             FROM usage WHERE model LIKE 'gemini%' AND ts >= ?1",
            rusqlite::params![day_start],
            |row| row.get(0),
        )?;
        self.gemini_tokens_today = tokens as u64;
        Ok(())
    }

    /// Expire CLI caps whose reset time has passed; should be called at session start.
    pub fn expire_stale_caps(&mut self) {
        let now = Utc::now().timestamp();
        if self.claude.cli_capped {
            if let Some(reset) = self.claude.cli_cap_reset_at {
                if now >= reset {
                    self.claude.cli_capped = false;
                    self.claude.cli_cap_reset_at = None;
                }
            }
        }
        // Also clear token-window data if the reset time has passed
        if let Some(reset) = self.claude.reset_at {
            if now >= reset {
                self.claude.tokens_remaining = None;
                self.claude.reset_at = None;
            }
        }
    }

    /// Returns a pressure value 0.0–1.0 for the given provider string.
    ///
    /// - `0.0`  = free headroom
    /// - `0.50` = half of the window consumed
    /// - `1.0`  = exhausted
    pub fn pressure(&self, provider: &str) -> f32 {
        match provider {
            "claude" | "anthropic" => {
                match (self.claude.tokens_remaining, self.claude.tokens_limit) {
                    (Some(rem), Some(lim)) if lim > 0 => {
                        let used = lim.saturating_sub(rem);
                        (used as f32) / (lim as f32)
                    }
                    _ => 0.0,
                }
            }
            "claude-cli" => {
                if self.claude.cli_capped {
                    return 1.0;
                }
                // No direct measurement — derive from API tracking as a proxy
                match (self.claude.tokens_remaining, self.claude.tokens_limit) {
                    (Some(rem), Some(lim)) if lim > 0 => {
                        let used = lim.saturating_sub(rem);
                        (used as f32) / (lim as f32)
                    }
                    _ => 0.0,
                }
            }
            "gemini" | "google" | "gemini-cli" => {
                if self.gemini_daily_budget > 0 {
                    (self.gemini_tokens_today as f32) / (self.gemini_daily_budget as f32)
                } else {
                    0.0
                }
            }
            _ => 0.0,
        }
    }

    /// True when rerouting away from `provider` makes sense for `task`.
    pub fn should_reroute(&self, provider: &str, task: &TaskType) -> bool {
        let p = self.pressure(provider);
        if p >= 0.95 {
            return true;
        }
        if p >= 0.80 {
            // Reroute for all tasks except the heaviest reasoning tasks
            return !matches!(task, TaskType::CodeReview | TaskType::ComplexReasoning);
        }
        if p >= 0.50 {
            // Reroute only lightweight tasks
            return matches!(
                task,
                TaskType::Documentation
                    | TaskType::QuickCompletion
                    | TaskType::WebSearch
                    | TaskType::Fallback
            );
        }
        false
    }

    /// A short human-readable summary of current pressure levels, printed at session start.
    pub fn status_line(&self) -> Option<String> {
        let cp = self.pressure("claude");
        let gp = self.pressure("gemini");
        let cc = self.claude.cli_capped;

        // Only show if something is actually notable (>50% or capped)
        if cp < 0.50 && gp < 0.50 && !cc {
            return None;
        }
        let mut parts = Vec::new();
        if cc {
            parts.push("claude-cli: CAPPED".to_string());
        } else if cp >= 0.50 {
            parts.push(format!("claude: {:.0}%", cp * 100.0));
        }
        if gp >= 0.50 {
            parts.push(format!("gemini: {:.0}%", gp * 100.0));
        }
        Some(format!("quota — {}", parts.join(", ")))
    }
}
