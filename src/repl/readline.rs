//! Crossterm-based line editor with slash-command and @-mention dropdown autocomplete.
//!
//! Render loop:
//!   1. MoveToColumn(0)               — go to start of current (input) row
//!   2. Clear(FromCursorDown)         — erase input line + all dropdown rows below
//!   3. Print prompt + buf            — redraw input
//!   4. Print N dropdown rows         — cursor is now N rows below input row
//!   5. MoveUp(N)                     — back to input row
//!   6. MoveToColumn(end_of_buf)      — cursor at end of typed text

use std::io::{self, Write as IoWrite};

use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute, queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, ClearType},
};

/// (slash-command, one-line description, takes_argument)
const COMMANDS: &[(&str, &str, bool)] = &[
    ("/agent",   "Toggle agentic mode (file tools)",                    false),
    ("/accept",  "Toggle auto-accept for write/run",                    false),
    ("/apply",   "Apply code blocks from last response",                false),
    ("/build",   "Run build pipeline  /build <desc>",                   true),
    ("/cheap",   "Force cheapest model  /cheap <query>",                true),
    ("/clear",   "Clear session context",                               false),
    ("/debate",  "Multi-model brainstorm  /debate [strategy] <query>",  true),
    ("/exit",    "End session",                                         false),
    ("/explain", "Show routing decision  /explain <query>",             true),
    ("/help",    "Show all commands",                                   false),
    ("/history", "Show conversation log with providers",                false),
    ("/index",   "Re-index current directory",                          false),
    ("/local",   "Force local model  /local <query>",                   true),
    ("/model",   "List or override model  /model <alias> <q>",          true),
    ("/persona", "Set developer persona  /persona [name|off]",          true),
    ("/scope",   "Set scope  /scope narrow|broad",                      true),
    ("/usage",   "Show token/cost usage",                               false),
];

const MAX_SHOW: usize = 8;

/// Read one line of input with dropdown autocomplete for `/` commands and `@` mentions.
/// `at_suggestions` is a list of `@mention` strings (e.g. `"@claude"`, `"@local/qwen2.5:7b"`).
/// Returns `None` on EOF (Ctrl+D on empty line) or Ctrl+C.
pub fn read_line(prompt: &str, at_suggestions: &[String]) -> Result<Option<String>> {
    let mut stdout = io::stdout();

    execute!(stdout, Print(prompt))?;
    terminal::enable_raw_mode()?;

    let mut buf = String::new();
    let mut cursor_pos: usize = 0;
    let mut selected: usize = 0;
    let mut suppress = false;

    let result = 'outer: loop {
        // ── Compute completions ──────────────────────────────────────────
        // completions: (display_text, description, takes_argument)
        let completions: Vec<(String, String, bool)> = if buf.starts_with('/') && !suppress {
            let prefix = buf.trim_end_matches(' ');
            COMMANDS
                .iter()
                .filter(|(cmd, _, _)| cmd.starts_with(prefix))
                .map(|&(cmd, desc, takes_arg)| (cmd.to_string(), desc.to_string(), takes_arg))
                .collect()
        } else if buf.starts_with('@') && !suppress && !at_suggestions.is_empty() {
            let prefix = buf.trim_end_matches(' ');
            at_suggestions
                .iter()
                .filter(|s| s.starts_with(prefix))
                .map(|s| {
                    // Provide a short description for well-known mentions
                    let desc = if s.starts_with("@local/") {
                        "route to this local model"
                    } else {
                        match s.as_str() {
                            "@claude"  => "route to Claude (CLI or API)",
                            "@gemini"  => "route to Gemini (CLI or API)",
                            "@local"   => "route to best local model",
                            "@cheap"   => "local first, then cheapest cloud",
                            "@fast"    => "local first, then fastest cloud",
                            _          => "",
                        }
                    };
                    (s.clone(), desc.to_string(), true)
                })
                .collect()
        } else {
            vec![]
        };

        let n = completions.len().min(MAX_SHOW);
        if n > 0 && selected >= n {
            selected = n - 1;
        }

        // ── Render ───────────────────────────────────────────────────────
        execute!(
            stdout,
            cursor::MoveToColumn(0),
            terminal::Clear(ClearType::FromCursorDown),
            Print(prompt),
            Print(&buf),
        )?;

        for (i, (cmd, desc, _)) in completions[..n].iter().enumerate() {
            queue!(stdout, Print("\r\n"))?;
            if i == selected {
                queue!(
                    stdout,
                    Print(format!("  \x1b[7m{:<26}{}\x1b[0m", cmd, desc)),
                )?;
            } else {
                queue!(
                    stdout,
                    SetForegroundColor(Color::DarkGrey),
                    Print(format!("  {:<26}{}", cmd, desc)),
                    ResetColor,
                )?;
            }
        }

        if n > 0 {
            execute!(stdout, cursor::MoveUp(n as u16))?;
        }
        let col = (prompt.len() + buf.chars().take(cursor_pos).count()) as u16;
        execute!(stdout, cursor::MoveToColumn(col))?;

        stdout.flush()?;

        // ── Handle input ─────────────────────────────────────────────────
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => match (key.code, key.modifiers) {

                (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => {
                    break 'outer None;
                }
                (KeyCode::Char('d'), m)
                    if m.contains(KeyModifiers::CONTROL) && buf.is_empty() =>
                {
                    break 'outer None;
                }

                (KeyCode::Enter, _) => {
                    if n > 0 {
                        let takes_arg = completions[selected].2;
                        let cmd = completions[selected].0.clone();
                        if takes_arg {
                            buf = format!("{cmd} ");
                            cursor_pos = buf.chars().count();
                            selected = 0;
                            suppress = false;
                        } else {
                            break 'outer Some(cmd);
                        }
                    } else {
                        break 'outer Some(buf.clone());
                    }
                }

                (KeyCode::Tab, _) => {
                    if n > 0 {
                        let takes_arg = completions[selected].2;
                        let cmd = completions[selected].0.clone();
                        buf = if takes_arg {
                            format!("{cmd} ")
                        } else {
                            cmd
                        };
                        cursor_pos = buf.chars().count();
                        selected = 0;
                        suppress = false;
                    }
                }

                (KeyCode::Up, _) => {
                    if selected > 0 {
                        selected -= 1;
                    }
                }
                (KeyCode::Down, _) => {
                    if n > 0 && selected + 1 < n {
                        selected += 1;
                    }
                }

                (KeyCode::Left, _) => {
                    if cursor_pos > 0 {
                        cursor_pos -= 1;
                    }
                }
                (KeyCode::Right, _) => {
                    if cursor_pos < buf.chars().count() {
                        cursor_pos += 1;
                    }
                }
                (KeyCode::Home, _) => {
                    cursor_pos = 0;
                }
                (KeyCode::End, _) => {
                    cursor_pos = buf.chars().count();
                }

                (KeyCode::Esc, _) => {
                    suppress = true;
                }

                (KeyCode::Backspace, _) => {
                    if cursor_pos > 0 {
                        let idx = buf.char_indices().nth(cursor_pos - 1).unwrap().0;
                        buf.remove(idx);
                        cursor_pos -= 1;
                        selected = 0;
                        suppress = false;
                    }
                }
                (KeyCode::Delete, _) => {
                    if cursor_pos < buf.chars().count() {
                        let idx = buf.char_indices().nth(cursor_pos).unwrap().0;
                        buf.remove(idx);
                        selected = 0;
                        suppress = false;
                    }
                }
                (KeyCode::Char(c), _) => {
                    let idx = buf.char_indices().nth(cursor_pos).map(|(i, _)| i).unwrap_or(buf.len());
                    buf.insert(idx, c);
                    cursor_pos += 1;
                    selected = 0;
                    suppress = false;
                }

                _ => {}
            },
            _ => {}
        }
    };

    execute!(
        stdout,
        terminal::Clear(ClearType::FromCursorDown),
        Print(if result.is_none() { "^C" } else { "" }),
        Print("\r\n"),
    )?;
    terminal::disable_raw_mode()?;
    stdout.flush()?;

    Ok(result)
}
