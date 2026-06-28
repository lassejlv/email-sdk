use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;

use crate::{EmailMessage, EmailSdkError};

pub type SharedEmailProvider = Arc<dyn EmailProvider>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmailProviderContext {
    pub idempotency_key: Option<String>,
    pub attempt: usize,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailProviderResponse {
    pub id: Option<String>,
    pub provider: String,
    pub message_id: Option<String>,
    pub accepted: Vec<String>,
    pub rejected: Vec<String>,
    pub raw: Option<String>,
}

impl EmailProviderResponse {
    pub fn new(provider: impl Into<String>) -> Self {
        Self {
            id: None,
            provider: provider.into(),
            message_id: None,
            accepted: Vec::new(),
            rejected: Vec::new(),
            raw: None,
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn message_id(mut self, message_id: impl Into<String>) -> Self {
        self.message_id = Some(message_id.into());
        self
    }
}

#[async_trait]
pub trait EmailProvider: Send + Sync {
    fn name(&self) -> &str;

    async fn send(
        &self,
        message: EmailMessage,
        context: EmailProviderContext,
    ) -> Result<EmailProviderResponse, EmailSdkError>;
}
