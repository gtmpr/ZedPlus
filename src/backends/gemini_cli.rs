use crate::backends::{
    AgentMessage, AgentTurn, Backend, BackendError, CompletionOptions, CompletionResult, ToolDef,
};
use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

pub struct GeminiCliBackend {
    bin: String,
}

impl GeminiCliBackend {
    pub fn new(bin: impl Into<String>) -> Self {
        Self { bin: bin.into() }
    }

    fn format_prompt(opts: &CompletionOptions) -> String {
        let mut out = String::new();
        if let Some(sys) = &opts.system {
            if !sys.is_empty() {
                out.push_str(sys);
                out.push_str("\n\n");
            }
        }
        for msg in &opts.messages {
            out.push_str(&msg.content);
            out.push('\n');
        }
        out.trim_end().to_string()
    }
}

#[async_trait]
impl Backend for GeminiCliBackend {
    fn name(&self) -> &str {
        "gemini-cli"
    }

    async fn complete(&self, opts: CompletionOptions) -> Result<CompletionResult, BackendError> {
        let prompt = Self::format_prompt(&opts);

        // Pass prompt via stdin to avoid Windows batch file argument length limits.
        let mut cmd = Command::new(&self.bin);
        cmd.stdin(std::process::Stdio::piped())
           .stdout(std::process::Stdio::piped())
           .stderr(std::process::Stdio::piped());
        if opts.auto_accept {
            cmd.arg("--yes");
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| BackendError::Other(anyhow::anyhow!("failed to run gemini: {e}")))?;

        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(prompt.as_bytes()).await;
        }

        let output = child.wait_with_output().await
            .map_err(|e| BackendError::Other(anyhow::anyhow!("failed to run gemini: {e}")))?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            let err_lower = err.to_lowercase();
            if err_lower.contains("usage limit") || err_lower.contains("rate limit")
                || err_lower.contains("too many requests") || err_lower.contains("quota")
                || err_lower.contains("resource_exhausted")
            {
                return Err(BackendError::RateLimit);
            }
            return Err(BackendError::Other(anyhow::anyhow!("gemini CLI: {err}")));
        }

        let content = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(CompletionResult {
            input_tokens: estimate_tokens(&prompt),
            output_tokens: estimate_tokens(&content),
            content,
            cache_hit: false,
        })
    }

    async fn complete_streaming(
        &self,
        opts: CompletionOptions,
        on_token: Box<dyn Fn(String) + Send>,
    ) -> Result<CompletionResult, BackendError> {
        let prompt = Self::format_prompt(&opts);
        let mut cmd = Command::new(&self.bin);
        cmd.stdin(std::process::Stdio::piped())
           .stdout(std::process::Stdio::piped())
           .stderr(std::process::Stdio::piped());
        if opts.auto_accept {
            cmd.arg("--yes");
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| BackendError::Other(anyhow::anyhow!("failed to run gemini: {e}")))?;

        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(prompt.as_bytes()).await;
        }

        let stdout = child.stdout.take().expect("stdout piped");
        let mut reader = BufReader::new(stdout);
        let mut content = String::new();
        let mut line = String::new();

        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => break,
                Ok(_) => {
                    on_token(line.clone());
                    content.push_str(&line);
                }
                Err(e) => return Err(BackendError::Other(e.into())),
            }
        }

        let mut stderr_handle = child.stderr.take();
        let status = child.wait().await.map_err(|e| BackendError::Other(e.into()))?;
        if !status.success() {
            let mut err_text = String::new();
            if let Some(ref mut se) = stderr_handle {
                use tokio::io::AsyncReadExt;
                let _ = se.read_to_string(&mut err_text).await;
            }
            let err_lower = err_text.to_lowercase();
            if err_lower.contains("usage limit") || err_lower.contains("rate limit")
                || err_lower.contains("too many requests") || err_lower.contains("quota")
                || err_lower.contains("resource_exhausted")
            {
                return Err(BackendError::RateLimit);
            }
            return Err(BackendError::Other(anyhow::anyhow!(
                "gemini CLI exited with {status}: {err_text}"
            )));
        }

        let content = content.trim().to_string();
        Ok(CompletionResult {
            input_tokens: estimate_tokens(&prompt),
            output_tokens: estimate_tokens(&content),
            content,
            cache_hit: false,
        })
    }

    async fn agent_step(
        &self,
        _system: Option<&str>,
        _messages: &[AgentMessage],
        _tools: &[ToolDef],
        _model_id: &str,
        _max_tokens: u32,
    ) -> Result<AgentTurn, BackendError> {
        Err(BackendError::Other(anyhow::anyhow!(
            "gemini-cli backend does not support tool use"
        )))
    }
}

fn estimate_tokens(text: &str) -> u32 {
    (text.len() as u32).saturating_add(3) / 4
}
