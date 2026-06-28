use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use email_sdk_core::{
    EmailMessage, EmailProvider, EmailProviderContext, EmailProviderResponse, EmailSdkError,
    SharedEmailProvider, http_error_message, is_retryable_status,
};

use super::payload::to_lettermint_payload;

#[derive(Debug, Clone)]
pub struct LettermintProviderOptions {
    pub api_token: String,
    pub base_url: String,
    pub route: Option<String>,
    pub headers: HashMap<String, String>,
    pub client: reqwest::Client,
}

impl LettermintProviderOptions {
    pub fn new(api_token: impl Into<String>) -> Self {
        Self {
            api_token: api_token.into(),
            base_url: "https://api.lettermint.co/v1".to_owned(),
            route: None,
            headers: HashMap::new(),
            client: reqwest::Client::new(),
        }
    }

    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn route(mut self, route: impl Into<String>) -> Self {
        self.route = Some(route.into());
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
pub struct LettermintProvider {
    options: LettermintProviderOptions,
}

impl LettermintProvider {
    pub fn new(options: LettermintProviderOptions) -> Self {
        Self { options }
    }

    pub fn shared(options: LettermintProviderOptions) -> SharedEmailProvider {
        Arc::new(Self::new(options))
    }

    pub fn base_url(&self) -> &str {
        &self.options.base_url
    }
}

pub fn lettermint(options: LettermintProviderOptions) -> Arc<LettermintProvider> {
    Arc::new(LettermintProvider::new(options))
}

#[async_trait]
impl EmailProvider for LettermintProvider {
    fn name(&self) -> &str {
        "lettermint"
    }

    async fn send(
        &self,
        message: EmailMessage,
        context: EmailProviderContext,
    ) -> Result<EmailProviderResponse, EmailSdkError> {
        let payload = to_lettermint_payload(&message, self.options.route.as_deref()).await?;
        let url = format!("{}/send", self.options.base_url.trim_end_matches('/'));
        let mut request = self
            .options
            .client
            .post(url)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("x-lettermint-token", &self.options.api_token)
            .json(&payload);

        for (name, value) in &self.options.headers {
            if name.eq_ignore_ascii_case("idempotency-key") {
                continue;
            }
            request = request.header(name, value);
        }

        if let Some(idempotency_key) = context.idempotency_key {
            request = request.header("Idempotency-Key", idempotency_key);
        }

        let response = request.send().await.map_err(|error| {
            EmailSdkError::provider_error(error.to_string(), "lettermint")
                .retryable(error.is_timeout() || error.is_connect())
        })?;

        let status = response.status();
        let body = read_response_body(response).await.map_err(|error| {
            EmailSdkError::provider_error(error.to_string(), "lettermint").retryable(false)
        })?;

        if !status.is_success() {
            return Err(EmailSdkError::provider_error(
                http_error_message("Lettermint", status.as_u16(), &body),
                "lettermint",
            )
            .status(status.as_u16())
            .retryable(is_retryable_status(status.as_u16()))
            .details(body.to_string()));
        }

        let id = first_string(&body, &["message_id"]);

        Ok(EmailProviderResponse {
            id: id.clone(),
            provider: "lettermint".to_owned(),
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

    use email_sdk_core::{EmailClientOptions, EmailMessage, SendOptions, create_email_client};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;

    #[tokio::test]
    async fn sends_http_request_to_lettermint() {
        let server = TestServer::start("HTTP/1.1 200 OK", r#"{"message_id":"lm_123"}"#).await;
        let provider = lettermint(
            LettermintProviderOptions::new("api_token")
                .base_url(server.base_url())
                .route("transactional")
                .header("Idempotency-Key", "static")
                .header("X-Test", "yes"),
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
        assert_eq!(response.message_id.as_deref(), Some("lm_123"));
        assert!(request.contains("post /send http/1.1"));
        assert!(request.contains("x-lettermint-token: api_token"));
        assert!(request.contains("idempotency-key: idem_123"));
        assert!(!request.contains("idempotency-key: static"));
        assert!(request.contains("x-test: yes"));
        assert!(request.contains("\"route\":\"transactional\""));
    }

    #[tokio::test]
    async fn maps_lettermint_error_response() {
        let server = TestServer::start(
            "HTTP/1.1 500 Internal Server Error",
            r#"{"message":"Down"}"#,
        )
        .await;
        let provider =
            lettermint(LettermintProviderOptions::new("api_token").base_url(server.base_url()));
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let error = client.send(message, None).await.unwrap_err();

        assert_eq!(error.provider.as_deref(), Some("lettermint"));
        assert_eq!(error.status, Some(500));
        assert!(error.retryable);
        assert_eq!(error.message, "Lettermint failed with 500: Down");
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
