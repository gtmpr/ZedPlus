use anyhow::Result;

pub fn bash_snippet() -> &'static str {
    r#"
# ZedPlus shell hotkey — Ctrl+Z to generate and run a shell command
_zedplus_shell_hotkey() {
    local desc
    read -r -p "ZedPlus shell: " desc
    if [ -n "$desc" ]; then
        zedplus shell "$desc"
    fi
}
bind -x '"\C-z": _zedplus_shell_hotkey'
"#
}

pub fn zsh_snippet() -> &'static str {
    r#"
# ZedPlus shell hotkey — Ctrl+Z to generate and run a shell command
_zedplus_shell_hotkey() {
    local desc
    vared -p "ZedPlus shell: " -c desc
    if [ -n "$desc" ]; then
        zedplus shell "$desc"
    fi
    zle reset-prompt
}
zle -N _zedplus_shell_hotkey
bindkey '^Z' _zedplus_shell_hotkey
"#
}

pub fn fish_snippet() -> &'static str {
    r#"
# ZedPlus shell hotkey — Ctrl+Z to generate and run a shell command
function _zedplus_shell_hotkey
    set desc (read --prompt-str "ZedPlus shell: ")
    if test -n "$desc"
        zedplus shell $desc
    end
    commandline -f repaint
end
bind \cz _zedplus_shell_hotkey
"#
}

/// Install the hotkey snippet interactively — detects shell from $SHELL, appends to RC file.
pub fn install_hotkey_interactive() -> Result<()> {
    let shell_bin = std::env::var("SHELL").unwrap_or_default();
    let shell_name = std::path::Path::new(&shell_bin)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("bash")
        .to_string();

    let (rc_file, snippet) = match shell_name.as_str() {
        "zsh" => ("~/.zshrc", zsh_snippet()),
        "fish" => ("~/.config/fish/config.fish", fish_snippet()),
        _ => ("~/.bashrc", bash_snippet()),
    };

    let expanded = shellexpand(rc_file);
    let snippet_trimmed = snippet.trim();

    // Check if already installed
    if let Ok(content) = std::fs::read_to_string(&expanded) {
        if content.contains("_zedplus_shell_hotkey") {
            println!("ZedPlus shell hotkey already installed in {rc_file}.");
            return Ok(());
        }
    }

    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&expanded)?;
    writeln!(file, "\n{snippet_trimmed}")?;

    println!("Installed ZedPlus shell hotkey (Ctrl+Z) in {rc_file}.");
    println!("Restart your shell or run: source {rc_file}");
    Ok(())
}

fn shellexpand(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    }
    path.to_string()
}
