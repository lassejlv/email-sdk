use std::sync::Arc;

use async_trait::async_trait;
use email_sdk_core::{
    EmailMessage, EmailProvider, EmailProviderContext, EmailProviderResponse, EmailSdkError,
    SharedEmailProvider, http_error_message, is_retryable_status,
};

use crate::payload::to_scaleway_payload;

#[derive(Debug, Clone)]
pub struct ScalewayProviderOptions {
    pub secret_key: String,
    pub project_id: String,
    pub region: String,
    pub base_url: String,
    pub client: reqwest::Client,
}

impl ScalewayProviderOptions {
    pub fn new(secret_key: impl Into<String>, project_id: impl Into<String>) -> Self {
        Self {
            secret_key: secret_key.into(),
            project_id: project_id.into(),
            region: "fr-par".to_owned(),
            base_url: "https://api.scaleway.com".to_owned(),
            client: reqwest::Client::new(),
        }
    }

    pub fn region(mut self, region: impl Into<String>) -> Self {
        self.region = region.into();
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
pub struct ScalewayProvider {
    options: ScalewayProviderOptions,
}

impl ScalewayProvider {
    pub fn new(options: ScalewayProviderOptions) -> Self {
        Self { options }
    }

    pub fn shared(options: ScalewayProviderOptions) -> SharedEmailProvider {
        Arc::new(Self::new(options))
    }

    pub fn base_url(&self) -> &str {
        &self.options.base_url
    }
}

pub fn scaleway(options: ScalewayProviderOptions) -> Arc<ScalewayProvider> {
    Arc::new(ScalewayProvider::new(options))
}

#[async_trait]
impl EmailProvider for ScalewayProvider {
    fn name(&self) -> &str {
        "scaleway"
    }

    async fn send(
        &self,
        message: EmailMessage,
        _context: EmailProviderContext,
    ) -> Result<EmailProviderResponse, EmailSdkError> {
        let payload = to_scaleway_payload(&message, &self.options.project_id).await?;
        let url = format!(
            "{}/transactional-email/v1alpha1/regions/{}/emails",
            self.options.base_url.trim_end_matches('/'),
            self.options.region
        );
        let response = self
            .options
            .client
            .post(url)
            .header("X-Auth-Token", &self.options.secret_key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|error| {
                EmailSdkError::provider_error(error.to_string(), "scaleway")
                    .retryable(error.is_timeout() || error.is_connect())
            })?;

        let status = response.status();
        let body = read_response_body(response).await.map_err(|error| {
            EmailSdkError::provider_error(error.to_string(), "scaleway").retryable(false)
        })?;

        if !status.is_success() {
            return Err(EmailSdkError::provider_error(
                http_error_message("scaleway", status.as_u16(), &body),
                "scaleway",
            )
            .status(status.as_u16())
            .retryable(is_retryable_status(status.as_u16()))
            .details(body.to_string()));
        }

        let id = first_string(&body, &["id", "email_id"]);

        Ok(EmailProviderResponse {
            id: id.clone(),
            provider: "scaleway".to_owned(),
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
    async fn sends_http_request_to_scaleway() {
        let server = TestServer::start("HTTP/1.1 200 OK", r#"{"id":"scw_123"}"#).await;
        let provider = scaleway(
            ScalewayProviderOptions::new("secret_key", "project_123")
                .base_url(server.base_url())
                .region("nl-ams"),
        );
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let response = client.send(message, None).await.unwrap();

        let request = server.request().await;
        assert_eq!(response.message_id.as_deref(), Some("scw_123"));
        assert!(
            request.contains("post /transactional-email/v1alpha1/regions/nl-ams/emails http/1.1")
        );
        assert!(request.contains("x-auth-token: secret_key"));
        assert!(request.contains("\"project_id\":\"project_123\""));
    }

    #[tokio::test]
    async fn parses_email_id_fallback() {
        let server = TestServer::start("HTTP/1.1 200 OK", r#"{"email_id":"scw_body_123"}"#).await;
        let provider = scaleway(
            ScalewayProviderOptions::new("secret_key", "project_123").base_url(server.base_url()),
        );
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let response = client.send(message, None).await.unwrap();

        assert_eq!(response.message_id.as_deref(), Some("scw_body_123"));
    }

    #[tokio::test]
    async fn maps_scaleway_error_response() {
        let server = TestServer::start(
            "HTTP/1.1 500 Internal Server Error",
            r#"{"message":"Down"}"#,
        )
        .await;
        let provider = scaleway(
            ScalewayProviderOptions::new("secret_key", "project_123").base_url(server.base_url()),
        );
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let error = client.send(message, None).await.unwrap_err();

        assert_eq!(error.provider.as_deref(), Some("scaleway"));
        assert_eq!(error.status, Some(500));
        assert!(error.retryable);
        assert_eq!(error.message, "scaleway failed with 500: Down");
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
