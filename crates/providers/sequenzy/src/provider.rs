use std::sync::Arc;

use async_trait::async_trait;
use email_sdk_core::{
    EmailMessage, EmailProvider, EmailProviderContext, EmailProviderResponse, EmailSdkError,
    SharedEmailProvider, http_error_message, is_retryable_status,
};

use crate::payload::to_sequenzy_payload;

#[derive(Debug, Clone)]
pub struct SequenzyProviderOptions {
    pub api_key: String,
    pub base_url: String,
    pub client: reqwest::Client,
}

impl SequenzyProviderOptions {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.sequenzy.com/api/v1".to_owned(),
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
pub struct SequenzyProvider {
    options: SequenzyProviderOptions,
}

impl SequenzyProvider {
    pub fn new(options: SequenzyProviderOptions) -> Self {
        Self { options }
    }

    pub fn shared(options: SequenzyProviderOptions) -> SharedEmailProvider {
        Arc::new(Self::new(options))
    }

    pub fn base_url(&self) -> &str {
        &self.options.base_url
    }
}

pub fn sequenzy(options: SequenzyProviderOptions) -> Arc<SequenzyProvider> {
    Arc::new(SequenzyProvider::new(options))
}

#[async_trait]
impl EmailProvider for SequenzyProvider {
    fn name(&self) -> &str {
        "sequenzy"
    }

    async fn send(
        &self,
        message: EmailMessage,
        _context: EmailProviderContext,
    ) -> Result<EmailProviderResponse, EmailSdkError> {
        let payload = to_sequenzy_payload(&message).await?;
        let url = format!(
            "{}/transactional/send",
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
                EmailSdkError::provider_error(error.to_string(), "sequenzy")
                    .retryable(error.is_timeout() || error.is_connect())
            })?;

        let status = response.status();
        let body = read_response_body(response).await.map_err(|error| {
            EmailSdkError::provider_error(error.to_string(), "sequenzy").retryable(false)
        })?;

        if !status.is_success() {
            return Err(EmailSdkError::provider_error(
                http_error_message("sequenzy", status.as_u16(), &body),
                "sequenzy",
            )
            .status(status.as_u16())
            .retryable(is_retryable_status(status.as_u16()))
            .details(body.to_string()));
        }

        if body.get("success").and_then(serde_json::Value::as_bool) == Some(false)
            || body.get("error").is_some()
        {
            return Err(
                EmailSdkError::provider_error(sequenzy_error_message(&body), "sequenzy")
                    .retryable(false)
                    .details(body.to_string()),
            );
        }

        let id = first_string(&body, &["jobId", "id"]);
        let accepted = accepted_recipients(&body).unwrap_or_default();

        Ok(EmailProviderResponse {
            id: id.clone(),
            provider: "sequenzy".to_owned(),
            message_id: id,
            accepted,
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

fn accepted_recipients(body: &serde_json::Value) -> Option<Vec<String>> {
    let value = body.as_object()?.get("to")?;
    if let Some(value) = value.as_str() {
        return Some(vec![value.to_owned()]);
    }

    value.as_array().map(|items| {
        items
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
            .collect()
    })
}

fn sequenzy_error_message(body: &serde_json::Value) -> String {
    let error = body
        .as_object()
        .and_then(|record| record.get("error"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("Unknown error");

    format!("Sequenzy failed: {error}")
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
    async fn sends_http_request_to_sequenzy() {
        let server = TestServer::start(
            "HTTP/1.1 200 OK",
            r#"{"success":true,"jobId":"job_123","to":["user@example.com"]}"#,
        )
        .await;
        let provider =
            sequenzy(SequenzyProviderOptions::new("api_key").base_url(server.base_url()));
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let response = client.send(message, None).await.unwrap();

        let request = server.request().await;
        assert_eq!(response.message_id.as_deref(), Some("job_123"));
        assert_eq!(response.accepted, vec!["user@example.com"]);
        assert!(request.contains("post /transactional/send http/1.1"));
        assert!(request.contains("authorization: bearer api_key"));
        assert!(request.contains("\"to\":\"to@example.com\""));
    }

    #[tokio::test]
    async fn maps_sequenzy_error_response() {
        let server =
            TestServer::start("HTTP/1.1 200 OK", r#"{"success":false,"error":"Rejected"}"#).await;
        let provider =
            sequenzy(SequenzyProviderOptions::new("api_key").base_url(server.base_url()));
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let error = client.send(message, None).await.unwrap_err();

        assert_eq!(error.provider.as_deref(), Some("sequenzy"));
        assert!(!error.retryable);
        assert_eq!(error.message, "Sequenzy failed: Rejected");
    }

    #[tokio::test]
    async fn maps_sequenzy_http_error_response() {
        let server = TestServer::start(
            "HTTP/1.1 500 Internal Server Error",
            r#"{"message":"Down"}"#,
        )
        .await;
        let provider =
            sequenzy(SequenzyProviderOptions::new("api_key").base_url(server.base_url()));
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let error = client.send(message, None).await.unwrap_err();

        assert_eq!(error.provider.as_deref(), Some("sequenzy"));
        assert_eq!(error.status, Some(500));
        assert!(error.retryable);
        assert_eq!(error.message, "sequenzy failed with 500: Down");
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
                let bytes_read = stream.read(&mut buffer).await.unwrap();
                let request = String::from_utf8_lossy(&buffer[..bytes_read]).to_lowercase();
                let _ = send_request.send(request);
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
