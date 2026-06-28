use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use email_sdk_core::{
    EmailMessage, EmailProvider, EmailProviderContext, EmailProviderResponse, EmailSdkError,
    SharedEmailProvider,
};

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryEmail {
    pub message: EmailMessage,
    pub response: EmailProviderResponse,
}

#[derive(Debug)]
pub struct MemoryProvider {
    name: String,
    sent: Mutex<Vec<MemoryEmail>>,
}

impl MemoryProvider {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            sent: Mutex::new(Vec::new()),
        }
    }

    pub fn shared(name: impl Into<String>) -> SharedEmailProvider {
        Arc::new(Self::new(name))
    }

    pub fn sent(&self) -> Vec<MemoryEmail> {
        self.sent
            .lock()
            .expect("memory provider lock poisoned")
            .clone()
    }

    pub fn clear(&self) {
        self.sent
            .lock()
            .expect("memory provider lock poisoned")
            .clear();
    }
}

impl Default for MemoryProvider {
    fn default() -> Self {
        Self::new("memory")
    }
}

#[async_trait]
impl EmailProvider for MemoryProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn send(
        &self,
        message: EmailMessage,
        _context: EmailProviderContext,
    ) -> Result<EmailProviderResponse, EmailSdkError> {
        let mut sent = self.sent.lock().expect("memory provider lock poisoned");
        let id = format!("mem_{}", sent.len() + 1);
        let response = EmailProviderResponse::new(self.name.clone())
            .id(id.clone())
            .message_id(id);

        sent.push(MemoryEmail {
            message,
            response: response.clone(),
        });

        Ok(response)
    }
}

pub fn memory_provider(name: impl Into<String>) -> Arc<MemoryProvider> {
    Arc::new(MemoryProvider::new(name))
}

#[cfg(test)]
mod tests {
    use email_sdk_core::{EmailClientOptions, EmailMessage, create_email_client};

    use super::*;

    #[tokio::test]
    async fn records_sent_messages() {
        let provider = memory_provider("memory");
        let client =
            create_email_client(EmailClientOptions::new().adapter(provider.clone())).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hello")
            .build();

        let response = client.send(message.clone(), None).await.unwrap();

        assert_eq!(response.provider, "memory");
        assert_eq!(provider.sent().len(), 1);
        assert_eq!(provider.sent()[0].message, message);
        assert_eq!(
            provider.sent()[0].response.message_id.as_deref(),
            Some("mem_1")
        );
    }
}
