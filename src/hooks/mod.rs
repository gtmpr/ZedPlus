use crate::config::schema::HooksConfig;

#[derive(Debug, Clone, Copy)]
pub enum HookPoint {
    BeforeApplyChange,
    AfterApplyChange,
    BeforeCommit,
    AfterCommit,
    BeforeSession,
    AfterSession,
    BeforeSearch,
    BeforeCloudSend,
}

pub struct HookRunner {
    config: HooksConfig,
}

impl HookRunner {
    pub fn new(config: &HooksConfig) -> Self {
        HookRunner { config: config.clone() }
    }

    /// Run the hook command for the given point. No-op when the hook is not configured.
    /// Blocks until the hook process exits. Returns Err if the hook exits non-zero.
    pub fn run(&self, point: HookPoint) -> anyhow::Result<()> {
        let cmd = match point {
            HookPoint::BeforeApplyChange => self.config.before_apply_change.as_deref(),
            HookPoint::AfterApplyChange  => self.config.after_apply_change.as_deref(),
            HookPoint::BeforeCommit      => self.config.before_commit.as_deref(),
            HookPoint::AfterCommit       => self.config.after_commit.as_deref(),
            HookPoint::BeforeSession     => self.config.before_session.as_deref(),
            HookPoint::AfterSession      => self.config.after_session.as_deref(),
            HookPoint::BeforeSearch      => self.config.before_search.as_deref(),
            HookPoint::BeforeCloudSend   => self.config.before_cloud_send.as_deref(),
        };

        let Some(cmd) = cmd else { return Ok(()); };

        let status = if cfg!(windows) {
            std::process::Command::new("cmd").args(["/C", cmd]).status()?
        } else {
            std::process::Command::new("sh").args(["-c", cmd]).status()?
        };

        if !status.success() {
            anyhow::bail!(
                "Hook {:?} failed with exit code {:?}",
                point,
                status.code()
            );
        }
        Ok(())
    }

    /// Run the hook; print a warning on failure instead of propagating the error.
    pub fn run_warn(&self, point: HookPoint) {
        if let Err(e) = self.run(point) {
            eprintln!("\x1b[33m[hook warning] {e}\x1b[0m");
        }
    }
}
