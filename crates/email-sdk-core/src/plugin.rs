use std::{collections::HashMap, sync::Arc};

use crate::{EmailProvider, EmailSdkError, SharedEmailProvider};

#[derive(Clone)]
pub struct EmailPluginContext {
    adapters: HashMap<String, SharedEmailProvider>,
    default_adapter: String,
    additions: Vec<SharedEmailProvider>,
}

impl EmailPluginContext {
    pub(crate) fn new(
        adapters: HashMap<String, SharedEmailProvider>,
        default_adapter: impl Into<String>,
    ) -> Self {
        Self {
            adapters,
            default_adapter: default_adapter.into(),
            additions: Vec::new(),
        }
    }

    pub fn adapters(&self) -> &HashMap<String, SharedEmailProvider> {
        &self.adapters
    }

    pub fn default_adapter(&self) -> &str {
        &self.default_adapter
    }

    pub fn add_adapter<T>(&mut self, adapter: Arc<T>)
    where
        T: EmailProvider + 'static,
    {
        self.additions.push(adapter);
    }

    pub fn add_shared_adapter(&mut self, adapter: SharedEmailProvider) {
        self.additions.push(adapter);
    }

    pub(crate) fn take_additions(self) -> Vec<SharedEmailProvider> {
        self.additions
    }
}

pub trait EmailPlugin: Send + Sync {
    fn id(&self) -> &str;

    fn install(&self, _context: &mut EmailPluginContext) -> Result<(), EmailSdkError> {
        Ok(())
    }

    fn hooks(&self) -> Vec<Arc<dyn crate::EmailHooks>> {
        Vec::new()
    }

    fn middleware(&self) -> Vec<Arc<dyn crate::EmailSendMiddleware>> {
        Vec::new()
    }
}
