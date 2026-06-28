use std::{sync::Arc, time::Duration};

use crate::{EmailSdkError, is_retryable_email_error};

pub type RetryDelay = Arc<dyn Fn(usize, &EmailSdkError) -> Duration + Send + Sync>;
pub type ShouldRetry = Arc<dyn Fn(&EmailSdkError, usize) -> bool + Send + Sync>;

#[derive(Clone)]
pub struct EmailRetryConfig {
    pub retries: usize,
    pub delay: RetryDelay,
    pub should_retry: ShouldRetry,
}

impl std::fmt::Debug for EmailRetryConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmailRetryConfig")
            .field("retries", &self.retries)
            .finish_non_exhaustive()
    }
}

impl Default for EmailRetryConfig {
    fn default() -> Self {
        Self {
            retries: 0,
            delay: Arc::new(default_delay),
            should_retry: Arc::new(|error, _attempt| is_retryable_email_error(error)),
        }
    }
}

pub fn default_delay(attempt: usize, _error: &EmailSdkError) -> Duration {
    let multiplier = 2u64.saturating_pow(attempt.saturating_sub(1) as u32);
    Duration::from_millis((100 * multiplier).min(2_000))
}
