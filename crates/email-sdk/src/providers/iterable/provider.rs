use std::sync::Arc;

use async_trait::async_trait;
use email_sdk_core::{
    EmailMessage, EmailProvider, EmailProviderContext, EmailProviderResponse, EmailSdkError,
    SharedEmailProvider, http_error_message, is_retryable_status,
};

use super::payload::{IterablePayloadOptions, to_iterable_payload};

#[derive(Debug, Clone)]
pub struct IterableProviderOptions {
    pub api_key: String,
    pub campaign_id: f64,
    pub allow_repeat_marketing_sends: Option<bool>,
    pub data_fields: serde_json::Map<String, serde_json::Value>,
    pub send_at: Option<String>,
    pub base_url: String,
    pub client: reqwest::Client,
}

impl IterableProviderOptions {
    pub fn new(api_key: impl Into<String>, campaign_id: f64) -> Result<Self, EmailSdkError> {
        if !campaign_id.is_finite() {
            return Err(EmailSdkError::validation(
                "iterable requires a numeric campaignId.",
            ));
        }

        Ok(Self {
            api_key: api_key.into(),
            campaign_id,
            allow_repeat_marketing_sends: None,
            data_fields: serde_json::Map::new(),
            send_at: None,
            base_url: "https://api.iterable.com".to_owned(),
            client: reqwest::Client::new(),
        })
    }

    pub fn allow_repeat_marketing_sends(mut self, value: bool) -> Self {
        self.allow_repeat_marketing_sends = Some(value);
        self
    }

    pub fn data_field(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.data_fields.insert(key.into(), value);
        self
    }

    pub fn send_at(mut self, send_at: impl Into<String>) -> Self {
        self.send_at = Some(send_at.into());
        self
    }

    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }
}

#[derive(Debug, Clone)]
pub struct IterableProvider {
    options: IterableProviderOptions,
}

impl IterableProvider {
    pub fn new(options: IterableProviderOptions) -> Self {
        Self { options }
    }

    pub fn shared(options: IterableProviderOptions) -> SharedEmailProvider {
        Arc::new(Self::new(options))
    }

    pub fn base_url(&self) -> &str {
        &self.options.base_url
    }
}

pub fn iterable(options: IterableProviderOptions) -> Arc<IterableProvider> {
    Arc::new(IterableProvider::new(options))
}

#[async_trait]
impl EmailProvider for IterableProvider {
    fn name(&self) -> &str {
        "iterable"
    }

    async fn send(
        &self,
        message: EmailMessage,
        _context: EmailProviderContext,
    ) -> Result<EmailProviderResponse, EmailSdkError> {
        let payload = to_iterable_payload(
            &message,
            &IterablePayloadOptions {
                campaign_id: self.options.campaign_id,
                allow_repeat_marketing_sends: self.options.allow_repeat_marketing_sends,
                data_fields: self.options.data_fields.clone(),
                send_at: self.options.send_at.clone(),
            },
        )?;
        let url = format!(
            "{}/api/email/target",
            self.options.base_url.trim_end_matches('/')
        );
        let response = self
            .options
            .client
            .post(url)
            .header("Api-Key", &self.options.api_key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|error| {
                EmailSdkError::provider_error(error.to_string(), "iterable")
                    .retryable(error.is_timeout() || error.is_connect())
            })?;

        let status = response.status();
        let body = read_response_body(response).await.map_err(|error| {
            EmailSdkError::provider_error(error.to_string(), "iterable").retryable(false)
        })?;

        if !status.is_success() {
            return Err(EmailSdkError::provider_error(
                http_error_message("iterable", status.as_u16(), &body),
                "iterable",
            )
            .status(status.as_u16())
            .retryable(is_retryable_status(status.as_u16()))
            .details(body.to_string()));
        }

        Ok(EmailProviderResponse {
            id: None,
            provider: "iterable".to_owned(),
            message_id: None,
            accepted: Vec::new(),
            rejected: Vec::new(),
            raw: Some(body.to_string()),
        })
    }
}

async fn read_response_body(
    response: reqwest::Response,
) -> Result<serde_json::Value, reqwest::Error> {
    let text = response.text().await?;
    if text.trim().is_empty() {
        return Ok(serde_json::Value::Object(serde_json::Map::new()));
    }

    Ok(serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text)))
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use email_sdk_core::{EmailClientOptions, EmailMessage, create_email_client};
    use serde_json::json;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;

    #[test]
    fn rejects_non_finite_campaign_id() {
        let error = IterableProviderOptions::new("api_key", f64::NAN).unwrap_err();

        assert_eq!(error.message, "iterable requires a numeric campaignId.");
    }

    #[tokio::test]
    async fn sends_http_request_to_iterable() {
        let server = TestServer::start("HTTP/1.1 200 OK", r#"{"ok":true}"#).await;
        let provider = iterable(
            IterableProviderOptions::new("api_key", 42.0)
                .unwrap()
                .allow_repeat_marketing_sends(true)
                .data_field("custom", json!("value"))
                .send_at("2026-06-28T12:00:00Z")
                .base_url(server.base_url()),
        );
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let response = client.send(message, None).await.unwrap();

        let request = server.request().await;
        assert_eq!(response.provider, "iterable");
        assert!(request.contains("POST /api/email/target HTTP/1.1"));
        assert!(request.contains("api-key: api_key"));
        assert!(request.contains("\"campaignId\":42.0"));
        assert!(request.contains("\"recipientEmail\":\"to@example.com\""));
        assert!(request.contains("\"allowRepeatMarketingSends\":true"));
        assert!(request.contains("\"sendAt\":\"2026-06-28T12:00:00Z\""));
        assert!(request.contains("\"custom\":\"value\""));
    }

    #[tokio::test]
    async fn maps_iterable_error_response() {
        let server = TestServer::start(
            "HTTP/1.1 500 Internal Server Error",
            r#"{"message":"Down"}"#,
        )
        .await;
        let provider = iterable(
            IterableProviderOptions::new("api_key", 42.0)
                .unwrap()
                .base_url(server.base_url()),
        );
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let error = client.send(message, None).await.unwrap_err();

        assert_eq!(error.provider.as_deref(), Some("iterable"));
        assert_eq!(error.status, Some(500));
        assert!(error.retryable);
        assert_eq!(error.message, "iterable failed with 500: Down");
    }

    struct TestServer {
        address: SocketAddr,
        request: tokio::sync::oneshot::Receiver<String>,
    }

    impl TestServer {
        async fn start(status: &str, body: &str) -> Self {
            let response = format!(
                "{status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let (send_request, request) = tokio::sync::oneshot::channel();

            tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buffer = vec![0; 8192];
                let read = stream.read(&mut buffer).await.unwrap();
                let raw_request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let _ = send_request.send(raw_request);
                stream.write_all(response.as_bytes()).await.unwrap();
            });

            Self { address, request }
        }

        fn base_url(&self) -> String {
            format!("http://{}", self.address)
        }

        async fn request(self) -> String {
            self.request.await.unwrap()
        }
    }
}
