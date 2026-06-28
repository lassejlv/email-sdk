use std::sync::Arc;

use async_trait::async_trait;
use email_sdk_core::{
    EmailMessage, EmailProvider, EmailProviderContext, EmailProviderResponse, EmailSdkError,
    SharedEmailProvider, http_error_message, is_retryable_status,
};

use crate::payload::to_sendgrid_payload;

#[derive(Debug, Clone)]
pub struct SendGridProviderOptions {
    pub api_key: String,
    pub base_url: String,
    pub client: reqwest::Client,
}

impl SendGridProviderOptions {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.sendgrid.com".to_owned(),
            client: reqwest::Client::new(),
        }
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
pub struct SendGridProvider {
    options: SendGridProviderOptions,
}

impl SendGridProvider {
    pub fn new(options: SendGridProviderOptions) -> Self {
        Self { options }
    }

    pub fn shared(options: SendGridProviderOptions) -> SharedEmailProvider {
        Arc::new(Self::new(options))
    }

    pub fn base_url(&self) -> &str {
        &self.options.base_url
    }
}

pub fn sendgrid(options: SendGridProviderOptions) -> Arc<SendGridProvider> {
    Arc::new(SendGridProvider::new(options))
}

#[async_trait]
impl EmailProvider for SendGridProvider {
    fn name(&self) -> &str {
        "sendgrid"
    }

    async fn send(
        &self,
        message: EmailMessage,
        _context: EmailProviderContext,
    ) -> Result<EmailProviderResponse, EmailSdkError> {
        let payload = to_sendgrid_payload(&message).await?;
        let url = format!(
            "{}/v3/mail/send",
            self.options.base_url.trim_end_matches('/')
        );
        let response = self
            .options
            .client
            .post(url)
            .bearer_auth(&self.options.api_key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|error| {
                EmailSdkError::provider_error(error.to_string(), "sendgrid")
                    .retryable(error.is_timeout() || error.is_connect())
            })?;

        let status = response.status();
        let message_id_header = response
            .headers()
            .get("x-message-id")
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        let body = read_response_body(response).await.map_err(|error| {
            EmailSdkError::provider_error(error.to_string(), "sendgrid").retryable(false)
        })?;

        if !status.is_success() {
            return Err(EmailSdkError::provider_error(
                http_error_message("sendgrid", status.as_u16(), &body),
                "sendgrid",
            )
            .status(status.as_u16())
            .retryable(is_retryable_status(status.as_u16()))
            .details(body.to_string()));
        }

        let id = first_string(&body, &["id", "message_id"]).or(message_id_header);

        Ok(EmailProviderResponse {
            id: id.clone(),
            provider: "sendgrid".to_owned(),
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
    async fn sends_http_request_to_sendgrid() {
        let server =
            TestServer::start("HTTP/1.1 202 Accepted", &[("x-message-id", "sg_123")], "").await;
        let provider =
            sendgrid(SendGridProviderOptions::new("api_key").base_url(server.base_url()));
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let response = client.send(message, None).await.unwrap();

        let request = server.request().await;
        assert_eq!(response.message_id.as_deref(), Some("sg_123"));
        assert!(request.contains("POST /v3/mail/send HTTP/1.1"));
        assert!(request.contains("authorization: Bearer api_key"));
        assert!(request.contains("\"from\":{\"email\":\"from@example.com\"}"));
        assert!(request.contains("\"to\":[{\"email\":\"to@example.com\"}]"));
    }

    #[tokio::test]
    async fn parses_sendgrid_json_message_id() {
        let server =
            TestServer::start("HTTP/1.1 200 OK", &[], r#"{"message_id":"sg_body_123"}"#).await;
        let provider =
            sendgrid(SendGridProviderOptions::new("api_key").base_url(server.base_url()));
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let response = client.send(message, None).await.unwrap();

        assert_eq!(response.message_id.as_deref(), Some("sg_body_123"));
    }

    #[tokio::test]
    async fn maps_sendgrid_error_response() {
        let server = TestServer::start(
            "HTTP/1.1 429 Too Many Requests",
            &[],
            r#"{"errors":[{"message":"Rate limited"}]}"#,
        )
        .await;
        let provider =
            sendgrid(SendGridProviderOptions::new("api_key").base_url(server.base_url()));
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let error = client.send(message, None).await.unwrap_err();

        assert_eq!(error.provider.as_deref(), Some("sendgrid"));
        assert_eq!(error.status, Some(429));
        assert!(error.retryable);
        assert_eq!(error.message, "sendgrid failed with 429: Rate limited");
    }

    struct TestServer {
        address: SocketAddr,
        request: tokio::sync::oneshot::Receiver<String>,
    }

    impl TestServer {
        async fn start(status: &str, headers: &[(&str, &str)], body: &str) -> Self {
            let mut response = format!(
                "{status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
                body.len()
            );
            for (name, value) in headers {
                response.push_str(&format!("{name}: {value}\r\n"));
            }
            response.push_str("\r\n");
            response.push_str(body);

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
