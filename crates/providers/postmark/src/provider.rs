use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use email_sdk_core::{
    EmailMessage, EmailProvider, EmailProviderContext, EmailProviderResponse, EmailSdkError,
    SharedEmailProvider, http_error_message, is_retryable_status,
};
use serde::Deserialize;

use crate::payload::to_postmark_payload;

#[derive(Debug, Clone)]
pub struct PostmarkProviderOptions {
    pub server_token: String,
    pub base_url: String,
    pub message_stream: Option<String>,
    pub headers: HashMap<String, String>,
    pub client: reqwest::Client,
}

impl PostmarkProviderOptions {
    pub fn new(server_token: impl Into<String>) -> Self {
        Self {
            server_token: server_token.into(),
            base_url: "https://api.postmarkapp.com".to_owned(),
            message_stream: None,
            headers: HashMap::new(),
            client: reqwest::Client::new(),
        }
    }

    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn message_stream(mut self, message_stream: impl Into<String>) -> Self {
        self.message_stream = Some(message_stream.into());
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
pub struct PostmarkProvider {
    options: PostmarkProviderOptions,
}

impl PostmarkProvider {
    pub fn new(options: PostmarkProviderOptions) -> Self {
        Self { options }
    }

    pub fn shared(options: PostmarkProviderOptions) -> SharedEmailProvider {
        Arc::new(Self::new(options))
    }

    pub fn base_url(&self) -> &str {
        &self.options.base_url
    }
}

pub fn postmark(options: PostmarkProviderOptions) -> Arc<PostmarkProvider> {
    Arc::new(PostmarkProvider::new(options))
}

#[async_trait]
impl EmailProvider for PostmarkProvider {
    fn name(&self) -> &str {
        "postmark"
    }

    async fn send(
        &self,
        message: EmailMessage,
        _context: EmailProviderContext,
    ) -> Result<EmailProviderResponse, EmailSdkError> {
        let payload = to_postmark_payload(&message, self.options.message_stream.as_deref()).await?;
        let url = format!("{}/email", self.options.base_url.trim_end_matches('/'));
        let mut request = self
            .options
            .client
            .post(url)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("X-Postmark-Server-Token", &self.options.server_token)
            .json(&payload);

        for (name, value) in &self.options.headers {
            request = request.header(name, value);
        }

        let response = request.send().await.map_err(|error| {
            EmailSdkError::provider_error(error.to_string(), "postmark")
                .retryable(error.is_timeout() || error.is_connect())
        })?;
        let status = response.status();
        let body = read_response_body(response).await.map_err(|error| {
            EmailSdkError::provider_error(error.to_string(), "postmark").retryable(false)
        })?;

        if !status.is_success() {
            return Err(EmailSdkError::provider_error(
                http_error_message("Postmark", status.as_u16(), &body),
                "postmark",
            )
            .status(status.as_u16())
            .retryable(is_retryable_status(status.as_u16()))
            .details(body.to_string()));
        }

        let parsed = serde_json::from_value::<PostmarkResponse>(body.clone()).map_err(|error| {
            EmailSdkError::provider_error(error.to_string(), "postmark").retryable(false)
        })?;

        Ok(EmailProviderResponse {
            id: parsed.message_id.clone(),
            provider: "postmark".to_owned(),
            message_id: parsed.message_id,
            accepted: parsed.to.into_iter().collect(),
            rejected: Vec::new(),
            raw: Some(body.to_string()),
        })
    }
}

#[derive(Debug, Deserialize)]
struct PostmarkResponse {
    #[serde(rename = "MessageID")]
    message_id: Option<String>,
    #[serde(rename = "To")]
    to: Option<String>,
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

    use email_sdk_core::{EmailClientOptions, EmailMessage, create_email_client};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;

    #[tokio::test]
    async fn sends_http_request_to_postmark() {
        let server = TestServer::start_json(
            "HTTP/1.1 200 OK",
            r#"{"MessageID":"msg_123","SubmittedAt":"now","To":"user@example.com"}"#,
        )
        .await;
        let provider = postmark(
            PostmarkProviderOptions::new("server_token")
                .base_url(server.base_url())
                .message_stream("outbound")
                .header("X-Extra", "yes"),
        );
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let response = client.send(message, None).await.unwrap();

        let request = server.request().await;
        assert_eq!(response.message_id.as_deref(), Some("msg_123"));
        assert_eq!(response.accepted.as_slice(), ["user@example.com"]);
        assert!(request.contains("POST /email HTTP/1.1"));
        assert!(request.contains("x-postmark-server-token: server_token"));
        assert!(request.contains("x-extra: yes"));
        assert!(request.contains("\"From\":\"from@example.com\""));
        assert!(request.contains("\"To\":\"to@example.com\""));
        assert!(request.contains("\"MessageStream\":\"outbound\""));
    }

    #[tokio::test]
    async fn maps_postmark_error_response() {
        let server = TestServer::start_json(
            "HTTP/1.1 422 Unprocessable Entity",
            r#"{"Message":"Bad email"}"#,
        )
        .await;
        let provider =
            postmark(PostmarkProviderOptions::new("server_token").base_url(server.base_url()));
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let error = client.send(message, None).await.unwrap_err();

        assert_eq!(error.provider.as_deref(), Some("postmark"));
        assert_eq!(error.status, Some(422));
        assert!(!error.retryable);
        assert_eq!(error.message, "Postmark failed with 422: Bad email");
    }

    struct TestServer {
        address: SocketAddr,
        request: tokio::sync::oneshot::Receiver<String>,
    }

    impl TestServer {
        async fn start_json(status: &str, body: &str) -> Self {
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
