use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use email_sdk_core::{
    EmailMessage, EmailProvider, EmailProviderContext, EmailProviderResponse, EmailSdkError,
    SharedEmailProvider, http_error_message, is_retryable_status,
};
use serde::Deserialize;

use super::payload::to_resend_payload;

#[derive(Debug, Clone)]
pub struct ResendProviderOptions {
    pub api_key: String,
    pub base_url: String,
    pub headers: HashMap<String, String>,
    pub client: reqwest::Client,
}

impl ResendProviderOptions {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.resend.com".to_owned(),
            headers: HashMap::new(),
            client: reqwest::Client::new(),
        }
    }

    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }

    pub fn client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }
}

#[derive(Debug, Clone)]
pub struct ResendProvider {
    options: ResendProviderOptions,
}

impl ResendProvider {
    pub fn new(options: ResendProviderOptions) -> Self {
        Self { options }
    }

    pub fn shared(options: ResendProviderOptions) -> SharedEmailProvider {
        Arc::new(Self::new(options))
    }

    pub fn base_url(&self) -> &str {
        &self.options.base_url
    }
}

pub fn resend(options: ResendProviderOptions) -> Arc<ResendProvider> {
    Arc::new(ResendProvider::new(options))
}

#[async_trait]
impl EmailProvider for ResendProvider {
    fn name(&self) -> &str {
        "resend"
    }

    async fn send(
        &self,
        message: EmailMessage,
        context: EmailProviderContext,
    ) -> Result<EmailProviderResponse, EmailSdkError> {
        let payload = to_resend_payload(&message).await?;
        let url = format!("{}/emails", self.options.base_url.trim_end_matches('/'));
        let mut request = self
            .options
            .client
            .post(url)
            .bearer_auth(&self.options.api_key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&payload);

        if let Some(idempotency_key) = context.idempotency_key {
            request = request.header("Idempotency-Key", idempotency_key);
        }

        for (name, value) in &self.options.headers {
            request = request.header(name, value);
        }

        let response = request.send().await.map_err(|error| {
            EmailSdkError::provider_error(error.to_string(), "resend")
                .retryable(error.is_timeout() || error.is_connect())
        })?;
        let status = response.status();
        let body = read_response_body(response).await.map_err(|error| {
            EmailSdkError::provider_error(error.to_string(), "resend").retryable(false)
        })?;

        if !status.is_success() {
            return Err(EmailSdkError::provider_error(
                http_error_message("Resend", status.as_u16(), &body),
                "resend",
            )
            .status(status.as_u16())
            .retryable(is_retryable_status(status.as_u16()))
            .details(body.to_string()));
        }

        let parsed = serde_json::from_value::<ResendResponse>(body.clone()).map_err(|error| {
            EmailSdkError::provider_error(error.to_string(), "resend").retryable(false)
        })?;

        Ok(EmailProviderResponse {
            id: parsed.id.clone(),
            provider: "resend".to_owned(),
            message_id: parsed.id,
            accepted: Vec::new(),
            rejected: Vec::new(),
            raw: Some(body.to_string()),
        })
    }
}

#[derive(Debug, Deserialize)]
struct ResendResponse {
    id: Option<String>,
}

async fn read_response_body(
    response: reqwest::Response,
) -> Result<serde_json::Value, reqwest::Error> {
    let text = response.text().await?;
    Ok(serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text)))
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use email_sdk_core::{EmailClientOptions, EmailMessage, SendOptions, create_email_client};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;

    #[tokio::test]
    async fn sends_http_request_to_resend() {
        let server = TestServer::start(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 18\r\n\r\n{\"id\":\"email_123\"}",
        )
        .await;
        let provider = resend(
            ResendProviderOptions::new("test_key")
                .base_url(server.base_url())
                .header("X-Extra", "yes"),
        );
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let response = client
            .send(
                message,
                Some(SendOptions::new().idempotency_key("idem_123")),
            )
            .await
            .unwrap();

        let request = server.request().await;
        assert_eq!(response.message_id.as_deref(), Some("email_123"));
        assert!(request.contains("POST /emails HTTP/1.1"));
        assert!(request.contains("authorization: Bearer test_key"));
        assert!(request.contains("idempotency-key: idem_123"));
        assert!(request.contains("x-extra: yes"));
        assert!(request.contains("\"from\":\"from@example.com\""));
        assert!(request.contains("\"to\":[\"to@example.com\"]"));
    }

    #[tokio::test]
    async fn maps_resend_error_response() {
        let server = TestServer::start(
            "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nContent-Length: 26\r\n\r\n{\"message\":\"Rate limited\"}",
        )
        .await;
        let provider = resend(ResendProviderOptions::new("test_key").base_url(server.base_url()));
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let error = client.send(message, None).await.unwrap_err();

        assert_eq!(error.provider.as_deref(), Some("resend"));
        assert_eq!(error.status, Some(429));
        assert!(error.retryable);
        assert_eq!(error.message, "Resend failed with 429: Rate limited");
    }

    #[tokio::test]
    async fn maps_plain_text_resend_error_response() {
        let server = TestServer::start(
            "HTTP/1.1 500 Internal Server Error\r\nContent-Type: text/plain\r\nContent-Length: 4\r\n\r\nnope",
        )
        .await;
        let provider = resend(ResendProviderOptions::new("test_key").base_url(server.base_url()));
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let error = client.send(message, None).await.unwrap_err();

        assert_eq!(error.status, Some(500));
        assert!(error.retryable);
        assert_eq!(error.message, "Resend failed with HTTP 500.");
    }

    struct TestServer {
        address: SocketAddr,
        request: tokio::sync::oneshot::Receiver<String>,
    }

    impl TestServer {
        async fn start(response: &'static str) -> Self {
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
