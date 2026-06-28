use std::{error::Error, fmt, sync::Arc};

#[derive(Debug, Clone)]
pub struct EmailSdkError {
    pub message: String,
    pub code: String,
    pub provider: Option<String>,
    pub status: Option<u16>,
    pub retryable: bool,
    pub details: Option<String>,
    pub cause: Option<Arc<dyn Error + Send + Sync>>,
}

impl EmailSdkError {
    pub fn new(message: impl Into<String>, code: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: code.into(),
            provider: None,
            status: None,
            retryable: false,
            details: None,
            cause: None,
        }
    }

    pub fn provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    pub fn status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    pub fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    pub fn details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }

    pub fn with_cause(mut self, cause: impl Error + Send + Sync + 'static) -> Self {
        self.cause = Some(Arc::new(cause));
        self
    }

    pub fn provider_error(message: impl Into<String>, provider: impl Into<String>) -> Self {
        Self::new(message, "provider_error").provider(provider)
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(message, "validation_error")
    }

    pub fn provider_not_found(provider: impl Into<String>) -> Self {
        let provider = provider.into();
        Self::new(
            format!("Email provider \"{provider}\" is not registered."),
            "provider_not_found",
        )
        .provider(provider)
    }
}

impl fmt::Display for EmailSdkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for EmailSdkError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.cause
            .as_deref()
            .map(|cause| cause as &(dyn Error + 'static))
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct EmailProviderError(pub EmailSdkError);

#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct EmailValidationError(pub EmailSdkError);

#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct EmailProviderNotFoundError(pub EmailSdkError);

impl From<EmailProviderError> for EmailSdkError {
    fn from(value: EmailProviderError) -> Self {
        value.0
    }
}

impl From<EmailValidationError> for EmailSdkError {
    fn from(value: EmailValidationError) -> Self {
        value.0
    }
}

impl From<EmailProviderNotFoundError> for EmailSdkError {
    fn from(value: EmailProviderNotFoundError) -> Self {
        value.0
    }
}

pub fn is_retryable_email_error(error: &EmailSdkError) -> bool {
    error.retryable
}
