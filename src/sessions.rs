/// Shared session persistence logic used by both the REPL and CLI commands.
use anyhow::Result;
use rusqlite::params;

use crate::backends::Message;

#[derive(Debug, Clone)]
pub struct ResumableSession {
    pub id: String,
    pub name: String,
    pub turn_count: i64,
    pub total_cost: f64,
    pub last_active: i64,
    pub git_branch: Option<String>,
}

impl ResumableSession {
    /// Human-readable label: "auth-refactor (7 turns, 2h ago)"
    pub fn label(&self) -> String {
        let age_secs = chrono::Utc::now().timestamp() - self.last_active;
        let age = human_age(age_secs);
        let branch = self
            .git_branch
            .as_deref()
            .map(|b| format!(" [{b}]"))
            .unwrap_or_default();
        format!("{}{} ({} turns, {})", self.name, branch, self.turn_count, age)
    }
}

/// Return active sessions for the given project, newest-first, within `since` epoch seconds.
/// Filters by branch if provided.
pub fn find_resumable(
    conn: &rusqlite::Connection,
    project_path: &str,
    branch: Option<&str>,
    since: i64,
    max: usize,
) -> Vec<ResumableSession> {
    let mut sessions = Vec::new();

    let sql = "SELECT id, COALESCE(name, id), turn_count, total_cost_usd, last_active, git_branch \
               FROM sessions \
               WHERE project_path = ?1 AND status = 'active' AND last_active >= ?2 \
               ORDER BY last_active DESC LIMIT ?3";

    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(_) => return sessions,
    };

    let rows = stmt.query_map(params![project_path, since, max as i64], |row| {
        Ok(ResumableSession {
            id: row.get(0)?,
            name: row.get(1)?,
            turn_count: row.get(2)?,
            total_cost: row.get(3)?,
            last_active: row.get(4)?,
            git_branch: row.get(5)?,
        })
    });

    if let Ok(iter) = rows {
        for r in iter.filter_map(|r| r.ok()) {
            // Optionally filter by branch
            if let Some(b) = branch {
                if r.git_branch.as_deref() != Some(b) {
                    continue;
                }
            }
            sessions.push(r);
        }
    }

    sessions
}

/// Load all turns for a session as a Vec<Message>, oldest first.
pub fn load_turns(conn: &rusqlite::Connection, session_id: &str) -> Vec<Message> {
    let sql = "SELECT role, content FROM session_turns WHERE session_id = ?1 ORDER BY ts ASC";
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    stmt.query_map(params![session_id], |row| {
        Ok(Message {
            role: row.get(0)?,
            content: row.get(1)?,
        })
    })
    .map(|iter| iter.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

/// List all sessions for a project (or all projects), sorted by last_active desc.
pub fn list_sessions(
    conn: &rusqlite::Connection,
    project_path: Option<&str>,
    limit: usize,
) -> Result<Vec<ResumableSession>> {
    let sql = match project_path {
        Some(_) => "SELECT id, COALESCE(name, id), turn_count, total_cost_usd, last_active, git_branch \
                    FROM sessions WHERE project_path = ?1 AND status = 'active' \
                    ORDER BY last_active DESC LIMIT ?2",
        None => "SELECT id, COALESCE(name, id), turn_count, total_cost_usd, last_active, git_branch \
                 FROM sessions WHERE status = 'active' ORDER BY last_active DESC LIMIT ?1",
    };

    let mut stmt = conn.prepare(sql)?;

    let rows = if let Some(path) = project_path {
        stmt.query_map(params![path, limit as i64], |row| {
            Ok(ResumableSession {
                id: row.get(0)?,
                name: row.get(1)?,
                turn_count: row.get(2)?,
                total_cost: row.get(3)?,
                last_active: row.get(4)?,
                git_branch: row.get(5)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect()
    } else {
        stmt.query_map(params![limit as i64], |row| {
            Ok(ResumableSession {
                id: row.get(0)?,
                name: row.get(1)?,
                turn_count: row.get(2)?,
                total_cost: row.get(3)?,
                last_active: row.get(4)?,
                git_branch: row.get(5)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect()
    };

    Ok(rows)
}

/// Update a session's name in the DB.
pub fn update_name(conn: &rusqlite::Connection, session_id: &str, name: &str) -> Result<()> {
    conn.execute(
        "UPDATE sessions SET name = ?1 WHERE id = ?2",
        params![name, session_id],
    )?;
    Ok(())
}

/// Offer a resume prompt via inquire. Returns the chosen session, or None for "start fresh".
pub fn offer_resume_prompt(candidates: Vec<ResumableSession>) -> Result<Option<ResumableSession>> {
    if candidates.is_empty() {
        return Ok(None);
    }

    if candidates.len() == 1 {
        let s = &candidates[0];
        let confirmed = inquire::Confirm::new(&format!("Resume '{}'? ({})", s.name, s.label()))
            .with_default(true)
            .prompt()
            .unwrap_or(false);
        return Ok(if confirmed { Some(candidates.into_iter().next().unwrap()) } else { None });
    }

    // Multiple candidates — use a picker
    let mut options: Vec<String> = candidates.iter().map(|s| s.label()).collect();
    options.push("[Start fresh session]".to_string());

    let choice = inquire::Select::new("Resume a previous session?", options.clone())
        .prompt()
        .unwrap_or_else(|_| "[Start fresh session]".to_string());

    if choice == "[Start fresh session]" {
        return Ok(None);
    }

    let idx = options.iter().position(|o| o == &choice).unwrap_or(0);
    if idx < candidates.len() {
        Ok(Some(candidates.into_iter().nth(idx).unwrap()))
    } else {
        Ok(None)
    }
}

fn human_age(secs: i64) -> String {
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}
