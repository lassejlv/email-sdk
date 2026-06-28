use std::collections::HashMap;

use async_trait::async_trait;

use crate::{EmailMessage, EmailProviderResponse, EmailSdkError};

#[derive(Debug, Clone)]
pub struct HookEvent {
    pub provider: String,
    pub message: EmailMessage,
    pub attempt: usize,
    pub metadata: HashMap<String, String>,
}

#[async_trait]
pub trait EmailHooks: Send + Sync {
    async fn before_send(&self, _event: HookEvent) -> Result<(), EmailSdkError> {
        Ok(())
    }

    async fn after_send(
        &self,
        _event: HookEvent,
        _response: EmailProviderResponse,
    ) -> Result<(), EmailSdkError> {
        Ok(())
    }

    async fn on_error(
        &self,
        _event: HookEvent,
        _error: EmailSdkError,
    ) -> Result<(), EmailSdkError> {
        Ok(())
    }

    async fn on_retry(
        &self,
        _event: HookEvent,
        _error: EmailSdkError,
        _next_attempt: usize,
        _delay_ms: u64,
    ) -> Result<(), EmailSdkError> {
        Ok(())
    }
}
