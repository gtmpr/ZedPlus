use std::process::Stdio;
use sysinfo::System;

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub total_ram_gb: f64,
    pub cpu_count: usize,
    pub vram_gb: Option<f64>,
    pub is_apple_silicon: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LocalLlmVerdict {
    Disabled { reason: String },
    CpuOnly { max_size: String },
    GpuSmall { max_size: String },
    GpuMedium { max_size: String },
    GpuLarge { max_size: String },
}

impl LocalLlmVerdict {
    pub fn can_train_lora(&self) -> bool {
        !matches!(self, LocalLlmVerdict::Disabled { .. } | LocalLlmVerdict::CpuOnly { .. })
    }

    pub fn suggested_model(&self) -> Option<&str> {
        match self {
            LocalLlmVerdict::Disabled { .. } => None,
            LocalLlmVerdict::CpuOnly { .. } => Some("llama3.2:3b"),
            LocalLlmVerdict::GpuSmall { .. } => Some("llama3.2:7b"),
            LocalLlmVerdict::GpuMedium { .. } => Some("llama3.2:8b"),
            LocalLlmVerdict::GpuLarge { .. } => Some("llama3.3:70b"),
        }
    }
}

pub fn scan() -> (DeviceInfo, LocalLlmVerdict) {
    let mut sys = System::new_all();
    sys.refresh_all();

    let total_ram_gb = sys.total_memory() as f64 / (1024.0 * 1024.0 * 1024.0);
    let cpu_count = sys.cpus().len();
    let vram_gb = detect_nvidia_vram();
    let is_apple_silicon = detect_apple_silicon();

    let info = DeviceInfo { total_ram_gb, cpu_count, vram_gb, is_apple_silicon };
    let verdict = compute_verdict(&info);
    (info, verdict)
}

fn compute_verdict(info: &DeviceInfo) -> LocalLlmVerdict {
    if info.total_ram_gb < 8.0 {
        return LocalLlmVerdict::Disabled {
            reason: format!("Only {:.0} GB RAM (minimum: 8 GB)", info.total_ram_gb),
        };
    }

    if info.is_apple_silicon {
        // Unified memory — use total_ram as effective VRAM
        return if info.total_ram_gb >= 32.0 {
            LocalLlmVerdict::GpuLarge { max_size: "30B+".into() }
        } else if info.total_ram_gb >= 16.0 {
            LocalLlmVerdict::GpuMedium { max_size: "13B".into() }
        } else {
            LocalLlmVerdict::GpuSmall { max_size: "7B Q4".into() }
        };
    }

    let vram = info.vram_gb.unwrap_or(0.0);
    if info.total_ram_gb >= 32.0 && vram >= 16.0 {
        LocalLlmVerdict::GpuLarge { max_size: "30B+".into() }
    } else if info.total_ram_gb >= 16.0 && vram >= 8.0 {
        LocalLlmVerdict::GpuMedium { max_size: "13B".into() }
    } else if vram >= 4.0 {
        LocalLlmVerdict::GpuSmall { max_size: "7B Q4 (GPU)".into() }
    } else {
        LocalLlmVerdict::CpuOnly { max_size: "7B Q4 (CPU, slow)".into() }
    }
}

fn detect_nvidia_vram() -> Option<f64> {
    // nvml-wrapper will be wired in Phase 9. Stub returns None.
    None
}

fn detect_apple_silicon() -> bool {
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("uname").arg("-m").output() {
            return String::from_utf8_lossy(&output.stdout).trim() == "arm64";
        }
    }
    false
}

// ── CLI tool detection ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CliDetection {
    pub claude: bool,
    pub gemini: bool,
    /// Actual binary name to invoke (may differ from "gemini" on Windows where it's "gemini.cmd")
    pub claude_bin: String,
    pub gemini_bin: String,
    /// Base URL for Ollama (e.g. "http://localhost:11434")
    pub ollama_url: String,
    /// Base URL for LM Studio (e.g. "http://localhost:1234")
    pub lmstudio_url: String,
    /// Models discovered from live local inference services (Ollama, LM Studio).
    /// Populated asynchronously after startup; empty until discovery runs.
    pub local_models: Vec<crate::local_models::DiscoveredModel>,
    pub openai_cli: bool,
    pub openai_bin: String,
    /// OpenAI Codex CLI (`codex` binary) detected
    pub codex_cli: bool,
    pub codex_bin: String,
    pub groq: bool,
    pub groq_bin: String,
    pub qwen: bool,
    pub qwen_bin: String,
    pub aider: bool,
    pub aider_bin: String,
}

impl Default for CliDetection {
    fn default() -> Self {
        Self {
            claude: false,
            gemini: false,
            claude_bin: "claude".into(),
            gemini_bin: "gemini".into(),
            ollama_url: "http://localhost:11434".into(),
            lmstudio_url: "http://localhost:1234".into(),
            local_models: Vec::new(),
            openai_cli: false,
            openai_bin: String::new(),
            codex_cli: false,
            codex_bin: String::new(),
            groq: false,
            groq_bin: String::new(),
            qwen: false,
            qwen_bin: String::new(),
            aider: false,
            aider_bin: String::new(),
        }
    }
}

pub fn detect_cli_tools() -> CliDetection {
    let (claude, claude_bin) = if probe_bin("claude") {
        (true, "claude".to_string())
    } else {
        (false, "claude".to_string())
    };

    let (gemini, gemini_bin) = if probe_bin("gemini") {
        (true, "gemini".to_string())
    } else if cfg!(windows) && probe_bin("gemini.cmd") {
        (true, "gemini.cmd".to_string())
    } else {
        (false, "gemini".to_string())
    };

    let (openai_cli, openai_bin) = if probe_bin("openai") {
        (true, "openai".to_string())
    } else {
        (false, String::new())
    };

    let (codex_cli, codex_bin) = if probe_bin("codex") {
        (true, "codex".to_string())
    } else {
        (false, String::new())
    };

    let (groq, groq_bin) = if probe_bin("groq") {
        (true, "groq".to_string())
    } else {
        (false, String::new())
    };

    let (qwen, qwen_bin) = if probe_bin("qwen") {
        (true, "qwen".to_string())
    } else {
        (false, String::new())
    };

    let (aider, aider_bin) = if probe_bin("aider") {
        (true, "aider".to_string())
    } else {
        (false, String::new())
    };

    CliDetection {
        claude, gemini, claude_bin, gemini_bin,
        ollama_url: "http://localhost:11434".into(),
        lmstudio_url: "http://localhost:1234".into(),
        local_models: Vec::new(),
        openai_cli, openai_bin,
        codex_cli, codex_bin,
        groq, groq_bin,
        qwen, qwen_bin,
        aider, aider_bin,
    }
}

fn probe_bin(name: &str) -> bool {
    std::process::Command::new(name)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}
