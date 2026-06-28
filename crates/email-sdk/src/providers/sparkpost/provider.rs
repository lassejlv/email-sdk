use std::sync::Arc;

use async_trait::async_trait;
use email_sdk_core::{
    EmailMessage, EmailProvider, EmailProviderContext, EmailProviderResponse, EmailSdkError,
    SharedEmailProvider, http_error_message, is_retryable_status,
};

use super::payload::to_sparkpost_payload;

#[derive(Debug, Clone)]
pub struct SparkPostProviderOptions {
    pub api_key: String,
    pub base_url: String,
    pub sandbox: Option<bool>,
    pub client: reqwest::Client,
}

impl SparkPostProviderOptions {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.sparkpost.com/api/v1".to_owned(),
            sandbox: None,
            client: reqwest::Client::new(),
        }
    }

    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn sandbox(mut self, sandbox: bool) -> Self {
        self.sandbox = Some(sandbox);
        self
    }

    pub fn client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }
}

#[derive(Debug, Clone)]
pub struct SparkPostProvider {
    options: SparkPostProviderOptions,
}

impl SparkPostProvider {
    pub fn new(options: SparkPostProviderOptions) -> Self {
        Self { options }
    }

    pub fn shared(options: SparkPostProviderOptions) -> SharedEmailProvider {
        Arc::new(Self::new(options))
    }

    pub fn base_url(&self) -> &str {
        &self.options.base_url
    }
}

pub fn sparkpost(options: SparkPostProviderOptions) -> Arc<SparkPostProvider> {
    Arc::new(SparkPostProvider::new(options))
}

#[async_trait]
impl EmailProvider for SparkPostProvider {
    fn name(&self) -> &str {
        "sparkpost"
    }

    async fn send(
        &self,
        message: EmailMessage,
        _context: EmailProviderContext,
    ) -> Result<EmailProviderResponse, EmailSdkError> {
        let payload = to_sparkpost_payload(&message, self.options.sandbox).await?;
        let url = format!(
            "{}/transmissions",
            self.options.base_url.trim_end_matches('/')
        );
        let response = self
            .options
            .client
            .post(url)
            .header(reqwest::header::AUTHORIZATION, &self.options.api_key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|error| {
                EmailSdkError::provider_error(error.to_string(), "sparkpost")
                    .retryable(error.is_timeout() || error.is_connect())
            })?;

        let status = response.status();
        let body = read_response_body(response).await.map_err(|error| {
            EmailSdkError::provider_error(error.to_string(), "sparkpost").retryable(false)
        })?;

        if !status.is_success() {
            return Err(EmailSdkError::provider_error(
                http_error_message("sparkpost", status.as_u16(), &body),
                "sparkpost",
            )
            .status(status.as_u16())
            .retryable(is_retryable_status(status.as_u16()))
            .details(body.to_string()));
        }

        let id = body
            .as_object()
            .and_then(|record| record.get("results"))
            .and_then(|results| first_string(results, &["id", "transmission_id"]));

        Ok(EmailProviderResponse {
            id: id.clone(),
            provider: "sparkpost".to_owned(),
            message_id: id,
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

fn first_string(body: &serde_json::Value, keys: &[&str]) -> Option<String> {
    let record = body.as_object()?;
    keys.iter()
        .find_map(|key| record.get(*key).and_then(serde_json::Value::as_str))
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use email_sdk_core::{EmailClientOptions, EmailMessage, create_email_client};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;

    #[tokio::test]
    async fn sends_http_request_to_sparkpost() {
        let server = TestServer::start("HTTP/1.1 200 OK", r#"{"results":{"id":"sp_123"}}"#).await;
        let provider = sparkpost(
            SparkPostProviderOptions::new("api_key")
                .base_url(server.base_url())
                .sandbox(true),
        );
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let response = client.send(message, None).await.unwrap();

        let request = server.request().await;
        assert_eq!(response.message_id.as_deref(), Some("sp_123"));
        assert!(request.contains("POST /transmissions HTTP/1.1"));
        assert!(request.contains("authorization: api_key"));
        assert!(request.contains("\"sandbox\":true"));
        assert!(request.contains("\"recipients\":[{\"address\":\"to@example.com\"}]"));
    }

    #[tokio::test]
    async fn parses_sparkpost_transmission_id_fallback() {
        let server = TestServer::start(
            "HTTP/1.1 200 OK",
            r#"{"results":{"transmission_id":"sp_body_123"}}"#,
        )
        .await;
        let provider =
            sparkpost(SparkPostProviderOptions::new("api_key").base_url(server.base_url()));
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let response = client.send(message, None).await.unwrap();

        assert_eq!(response.message_id.as_deref(), Some("sp_body_123"));
    }

    #[tokio::test]
    async fn maps_sparkpost_error_response() {
        let server = TestServer::start(
            "HTTP/1.1 429 Too Many Requests",
            r#"{"errors":[{"message":"Rate limited"}]}"#,
        )
        .await;
        let provider =
            sparkpost(SparkPostProviderOptions::new("api_key").base_url(server.base_url()));
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let error = client.send(message, None).await.unwrap_err();

        assert_eq!(error.provider.as_deref(), Some("sparkpost"));
        assert_eq!(error.status, Some(429));
        assert!(error.retryable);
        assert_eq!(error.message, "sparkpost failed with 429: Rate limited");
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
