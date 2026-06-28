use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use email_sdk_core::{
    EmailMessage, EmailProvider, EmailProviderContext, EmailProviderResponse, EmailSdkError,
    SharedEmailProvider, http_error_message, is_retryable_status,
};

use super::payload::to_primitive_payload;

#[derive(Debug, Clone)]
pub struct PrimitiveProviderOptions {
    pub api_key: String,
    pub base_url: String,
    pub headers: HashMap<String, String>,
    pub client: reqwest::Client,
}

impl PrimitiveProviderOptions {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.primitive.dev/v1".to_owned(),
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
pub struct PrimitiveProvider {
    options: PrimitiveProviderOptions,
}

impl PrimitiveProvider {
    pub fn new(options: PrimitiveProviderOptions) -> Self {
        Self { options }
    }

    pub fn shared(options: PrimitiveProviderOptions) -> SharedEmailProvider {
        Arc::new(Self::new(options))
    }

    pub fn base_url(&self) -> &str {
        &self.options.base_url
    }
}

pub fn primitive(options: PrimitiveProviderOptions) -> Arc<PrimitiveProvider> {
    Arc::new(PrimitiveProvider::new(options))
}

#[async_trait]
impl EmailProvider for PrimitiveProvider {
    fn name(&self) -> &str {
        "primitive"
    }

    async fn send(
        &self,
        message: EmailMessage,
        context: EmailProviderContext,
    ) -> Result<EmailProviderResponse, EmailSdkError> {
        let payload = to_primitive_payload(&message).await?;
        let url = format!("{}/send-mail", self.options.base_url.trim_end_matches('/'));
        let mut request = self
            .options
            .client
            .post(url)
            .bearer_auth(&self.options.api_key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
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
            EmailSdkError::provider_error(error.to_string(), "primitive")
                .retryable(error.is_timeout() || error.is_connect())
        })?;

        let status = response.status();
        let body = read_response_body(response).await.map_err(|error| {
            EmailSdkError::provider_error(error.to_string(), "primitive").retryable(false)
        })?;

        if !status.is_success() {
            return Err(EmailSdkError::provider_error(
                primitive_error_message(status.as_u16(), &body),
                "primitive",
            )
            .status(status.as_u16())
            .retryable(is_retryable_status(status.as_u16()))
            .details(body.to_string()));
        }

        let data = body.as_object().and_then(|record| record.get("data"));
        let id = data.and_then(|data| first_string(data, &["id"]));
        let accepted = data
            .and_then(|data| string_array(data, "accepted"))
            .unwrap_or_default();
        let rejected = data
            .and_then(|data| string_array(data, "rejected"))
            .unwrap_or_default();

        Ok(EmailProviderResponse {
            id: id.clone(),
            provider: "primitive".to_owned(),
            message_id: id,
            accepted,
            rejected,
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

fn string_array(body: &serde_json::Value, key: &str) -> Option<Vec<String>> {
    body.as_object()?.get(key)?.as_array().map(|items| {
        items
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
            .collect()
    })
}

fn primitive_error_message(status: u16, body: &serde_json::Value) -> String {
    if let Some(error) = body.as_object().and_then(|record| record.get("error"))
        && let Some(error) = error.as_object()
        && let Some(message) = error.get("message").and_then(serde_json::Value::as_str)
    {
        return match error.get("code").and_then(serde_json::Value::as_str) {
            Some(code) => format!("Primitive failed with {status}: {message} ({code})"),
            None => format!("Primitive failed with {status}: {message}"),
        };
    }

    http_error_message("Primitive", status, body)
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
    async fn sends_http_request_to_primitive() {
        let server = TestServer::start(
            "HTTP/1.1 200 OK",
            r#"{"data":{"id":"prim_123","accepted":["to@example.com"],"rejected":["bad@example.com"]}}"#,
        )
        .await;
        let provider = primitive(
            PrimitiveProviderOptions::new("api_key")
                .base_url(server.base_url())
                .header("X-Static", "yes")
                .header("Idempotency-Key", "static"),
        );
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let response = client
            .send(
                message,
                Some(SendOptions::new().idempotency_key("send_key")),
            )
            .await
            .unwrap();

        let request = server.request().await;
        assert_eq!(response.message_id.as_deref(), Some("prim_123"));
        assert_eq!(response.accepted.as_slice(), ["to@example.com"]);
        assert_eq!(response.rejected.as_slice(), ["bad@example.com"]);
        assert!(request.contains("POST /send-mail HTTP/1.1"));
        assert!(request.contains("authorization: Bearer api_key"));
        assert!(request.contains("x-static: yes"));
        assert!(request.contains("idempotency-key: send_key"));
        assert!(!request.contains("idempotency-key: static"));
        assert!(request.contains("\"to\":\"to@example.com\""));
    }

    #[tokio::test]
    async fn maps_primitive_custom_error_response() {
        let server = TestServer::start(
            "HTTP/1.1 422 Unprocessable Entity",
            r#"{"error":{"code":"bad_request","message":"Nope"}}"#,
        )
        .await;
        let provider =
            primitive(PrimitiveProviderOptions::new("api_key").base_url(server.base_url()));
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let error = client.send(message, None).await.unwrap_err();

        assert_eq!(error.provider.as_deref(), Some("primitive"));
        assert_eq!(error.status, Some(422));
        assert!(!error.retryable);
        assert_eq!(
            error.message,
            "Primitive failed with 422: Nope (bad_request)"
        );
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
