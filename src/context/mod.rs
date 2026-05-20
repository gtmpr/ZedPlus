pub mod locale;
pub mod project;

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

        format!(
            "{prefix}\n{scope_instruction}\n"
        )
    }
}
