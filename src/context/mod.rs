pub mod locale;
pub mod project;
pub mod zedplusmd;

use crate::config::schema::Config;
use locale::LocaleContext;

pub struct SystemPromptBuilder<'a> {
    config: &'a Config,
}

impl<'a> SystemPromptBuilder<'a> {
    pub fn new(config: &'a Config) -> Self {
        Self { config }
    }

    pub fn build_prefix(&self) -> String {
        let locale_ctx = LocaleContext::new(self.config.locale.clone());
        locale_ctx.system_prompt_prefix()
    }

    pub fn build_base_system_prompt(&self) -> String {
        let prefix = self.build_prefix();
        let scope_instruction = match self.config.behavior.default_scope {
            crate::config::schema::Scope::Narrow => {
                "Answer only the specific question asked. Do not refactor, rename, reorganize, or modify code beyond what was explicitly requested. If you notice adjacent issues, mention them briefly at the end — do not fix them."
            }
            crate::config::schema::Scope::Broad => {
                "You may suggest and implement adjacent improvements when relevant, but always show a diff before applying any change."
            }
        };

        let mut prompt = format!("{prefix}\n{scope_instruction}\n");

        // Inject ZEDPLUS.md project context when present
        if let Some(zmd) = std::env::current_dir()
            .ok()
            .and_then(|cwd| zedplusmd::load(&cwd))
        {
            prompt.push_str("\n\n## Project Context (ZEDPLUS.md)\n\n");
            prompt.push_str(&zmd);
        }

        prompt
    }
}
