use std::sync::Arc;

use async_trait::async_trait;
use email_sdk_core::{
    EmailMessage, EmailProvider, EmailProviderContext, EmailProviderResponse, EmailSdkError,
    SharedEmailProvider, http_error_message, is_retryable_status,
};

use crate::payload::to_cloudflare_payload;

#[derive(Debug, Clone)]
pub struct CloudflareProviderOptions {
    pub api_token: String,
    pub account_id: String,
    pub base_url: String,
    pub client: reqwest::Client,
}

impl CloudflareProviderOptions {
    pub fn new(api_token: impl Into<String>, account_id: impl Into<String>) -> Self {
        Self {
            api_token: api_token.into(),
            account_id: account_id.into(),
            base_url: "https://api.cloudflare.com/client/v4".to_owned(),
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
pub struct CloudflareProvider {
    options: CloudflareProviderOptions,
}

impl CloudflareProvider {
    pub fn new(options: CloudflareProviderOptions) -> Self {
        Self { options }
    }

    pub fn shared(options: CloudflareProviderOptions) -> SharedEmailProvider {
        Arc::new(Self::new(options))
    }

    pub fn base_url(&self) -> &str {
        &self.options.base_url
    }

    pub fn account_id(&self) -> &str {
        &self.options.account_id
    }
}

pub fn cloudflare(options: CloudflareProviderOptions) -> Arc<CloudflareProvider> {
    Arc::new(CloudflareProvider::new(options))
}

#[async_trait]
impl EmailProvider for CloudflareProvider {
    fn name(&self) -> &str {
        "cloudflare"
    }

    async fn send(
        &self,
        message: EmailMessage,
        _context: EmailProviderContext,
    ) -> Result<EmailProviderResponse, EmailSdkError> {
        let payload = to_cloudflare_payload(&message).await?;
        let url = format!(
            "{}/accounts/{}/email/sending/send",
            self.options.base_url.trim_end_matches('/'),
            self.options.account_id
        );
        let response = self
            .options
            .client
            .post(url)
            .bearer_auth(&self.options.api_token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|error| {
                EmailSdkError::provider_error(error.to_string(), "cloudflare")
                    .retryable(error.is_timeout() || error.is_connect())
            })?;

        let status = response.status();
        let body = read_response_body(response).await.map_err(|error| {
            EmailSdkError::provider_error(error.to_string(), "cloudflare").retryable(false)
        })?;

        if !status.is_success() {
            return Err(EmailSdkError::provider_error(
                http_error_message("cloudflare", status.as_u16(), &body),
                "cloudflare",
            )
            .status(status.as_u16())
            .retryable(is_retryable_status(status.as_u16()))
            .details(body.to_string()));
        }

        if body.get("success").and_then(serde_json::Value::as_bool) != Some(true) {
            return Err(EmailSdkError::provider_error(
                cloudflare_error_message(&body),
                "cloudflare",
            )
            .retryable(false)
            .details(body.to_string()));
        }

        let result = body.as_object().and_then(|record| record.get("result"));
        let accepted = result
            .map(|result| {
                let mut accepted = string_array(result, "delivered").unwrap_or_default();
                accepted.extend(string_array(result, "queued").unwrap_or_default());
                accepted
            })
            .unwrap_or_default();
        let rejected = result
            .and_then(|result| string_array(result, "permanent_bounces"))
            .unwrap_or_default();

        Ok(EmailProviderResponse {
            id: None,
            provider: "cloudflare".to_owned(),
            message_id: None,
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

fn string_array(body: &serde_json::Value, key: &str) -> Option<Vec<String>> {
    body.as_object()?.get(key)?.as_array().map(|items| {
        items
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
            .collect()
    })
}

fn cloudflare_error_message(body: &serde_json::Value) -> String {
    body.as_object()
        .and_then(|record| record.get("errors"))
        .and_then(serde_json::Value::as_array)
        .and_then(|errors| {
            errors.iter().find_map(|error| {
                error
                    .as_object()
                    .and_then(|record| record.get("message"))
                    .and_then(serde_json::Value::as_str)
            })
        })
        .map(|message| format!("cloudflare failed: {message}"))
        .unwrap_or_else(|| "cloudflare failed.".to_owned())
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
    async fn sends_http_request_to_cloudflare() {
        let server = TestServer::start(
            "HTTP/1.1 200 OK",
            r#"{"success":true,"result":{"delivered":["a@example.com"],"queued":["b@example.com"],"permanent_bounces":["c@example.com"]}}"#,
        )
        .await;
        let provider = cloudflare(
            CloudflareProviderOptions::new("api_token", "account_123").base_url(server.base_url()),
        );
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let response = client.send(message, None).await.unwrap();

        let request = server.request().await;
        assert_eq!(response.accepted, vec!["a@example.com", "b@example.com"]);
        assert_eq!(response.rejected, vec!["c@example.com"]);
        assert!(request.contains("post /accounts/account_123/email/sending/send http/1.1"));
        assert!(request.contains("authorization: bearer api_token"));
        assert!(request.contains("\"from\":\"from@example.com\""));
        assert!(request.contains("\"to\":[\"to@example.com\"]"));
    }

    #[tokio::test]
    async fn maps_cloudflare_success_false_response() {
        let server = TestServer::start(
            "HTTP/1.1 200 OK",
            r#"{"success":false,"errors":[{"message":"Rejected"}]}"#,
        )
        .await;
        let provider = cloudflare(
            CloudflareProviderOptions::new("api_token", "account_123").base_url(server.base_url()),
        );
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let error = client.send(message, None).await.unwrap_err();

        assert_eq!(error.provider.as_deref(), Some("cloudflare"));
        assert!(!error.retryable);
        assert_eq!(error.message, "cloudflare failed: Rejected");
    }

    #[tokio::test]
    async fn maps_cloudflare_http_error_response() {
        let server = TestServer::start(
            "HTTP/1.1 500 Internal Server Error",
            r#"{"message":"Down"}"#,
        )
        .await;
        let provider = cloudflare(
            CloudflareProviderOptions::new("api_token", "account_123").base_url(server.base_url()),
        );
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let error = client.send(message, None).await.unwrap_err();

        assert_eq!(error.provider.as_deref(), Some("cloudflare"));
        assert_eq!(error.status, Some(500));
        assert!(error.retryable);
        assert_eq!(error.message, "cloudflare failed with 500: Down");
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
