use std::collections::HashMap;

use async_trait::async_trait;

use crate::{EmailMessage, EmailProviderResponse, EmailSdkError};

#[derive(Debug, Clone)]
pub struct BeforeSendEvent {
    pub message: EmailMessage,
    pub options: Option<crate::SendOptions>,
}

#[derive(Debug, Clone, Default)]
pub struct BeforeSendResult {
    pub message: Option<EmailMessage>,
    pub options: Option<crate::SendOptions>,
}

#[derive(Debug, Clone)]
pub struct SendSuccessEvent {
    pub provider: String,
    pub message: EmailMessage,
    pub attempt: usize,
    pub metadata: HashMap<String, String>,
    pub response: EmailProviderResponse,
}

#[derive(Debug, Clone)]
pub struct SendFailureEvent {
    pub provider: String,
    pub message: EmailMessage,
    pub attempt: usize,
    pub metadata: HashMap<String, String>,
    pub error: EmailSdkError,
}

#[async_trait]
pub trait EmailSendMiddleware: Send + Sync {
    async fn before_send(
        &self,
        _event: BeforeSendEvent,
    ) -> Result<Option<BeforeSendResult>, EmailSdkError> {
        Ok(None)
    }

    async fn after_send(&self, _event: SendSuccessEvent) -> Result<(), EmailSdkError> {
        Ok(())
    }

    async fn on_error(&self, _event: SendFailureEvent) -> Result<(), EmailSdkError> {
        Ok(())
    }
}
