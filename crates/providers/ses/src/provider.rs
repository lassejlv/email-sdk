use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use chrono::Utc;
use email_sdk_core::{
    EmailMessage, EmailProvider, EmailProviderContext, EmailProviderResponse, EmailSdkError,
    SharedEmailProvider, http_error_message, is_retryable_status,
};
use serde::Deserialize;

use crate::{
    payload::{SesPayloadOptions, to_ses_payload},
    signing::{AwsCredentials, AwsSignRequest, sign_aws_request},
};

#[derive(Debug, Clone)]
pub struct SesProviderOptions {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub region: String,
    pub session_token: Option<String>,
    pub base_url: String,
    pub charset: Option<String>,
    pub configuration_set_name: Option<String>,
    pub client: reqwest::Client,
}

impl SesProviderOptions {
    pub fn new(
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
        region: impl Into<String>,
    ) -> Self {
        let region = region.into();
        Self {
            access_key_id: access_key_id.into(),
            secret_access_key: secret_access_key.into(),
            base_url: format!("https://email.{region}.amazonaws.com"),
            region,
            session_token: None,
            charset: None,
            configuration_set_name: None,
            client: reqwest::Client::new(),
        }
    }

    pub fn session_token(mut self, session_token: impl Into<String>) -> Self {
        self.session_token = Some(session_token.into());
        self
    }

    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn charset(mut self, charset: impl Into<String>) -> Self {
        self.charset = Some(charset.into());
        self
    }

    pub fn configuration_set_name(mut self, configuration_set_name: impl Into<String>) -> Self {
        self.configuration_set_name = Some(configuration_set_name.into());
        self
    }

    pub fn client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }
}

#[derive(Debug, Clone)]
pub struct SesProvider {
    options: SesProviderOptions,
}

impl SesProvider {
    pub fn new(options: SesProviderOptions) -> Self {
        Self { options }
    }

    pub fn shared(options: SesProviderOptions) -> SharedEmailProvider {
        Arc::new(Self::new(options))
    }

    pub fn base_url(&self) -> &str {
        &self.options.base_url
    }

    pub fn region(&self) -> &str {
        &self.options.region
    }
}

pub fn ses(options: SesProviderOptions) -> Arc<SesProvider> {
    Arc::new(SesProvider::new(options))
}

#[async_trait]
impl EmailProvider for SesProvider {
    fn name(&self) -> &str {
        "ses"
    }

    async fn send(
        &self,
        message: EmailMessage,
        _context: EmailProviderContext,
    ) -> Result<EmailProviderResponse, EmailSdkError> {
        let endpoint = reqwest::Url::parse(&self.options.base_url)
            .and_then(|url| url.join("/v2/email/outbound-emails"))
            .map_err(|error| EmailSdkError::provider_error(error.to_string(), "ses"))?;
        let payload = to_ses_payload(
            &message,
            &SesPayloadOptions {
                charset: self.options.charset.clone(),
                configuration_set_name: self.options.configuration_set_name.clone(),
            },
        )
        .await?;
        let body = serde_json::to_string(&payload)
            .map_err(|error| EmailSdkError::provider_error(error.to_string(), "ses"))?;
        let credentials = AwsCredentials {
            access_key_id: self.options.access_key_id.clone(),
            secret_access_key: self.options.secret_access_key.clone(),
            session_token: self.options.session_token.clone(),
        };
        let signed_headers = sign_aws_request(AwsSignRequest {
            credentials: &credentials,
            region: &self.options.region,
            service: "ses",
            method: "POST",
            url: &endpoint,
            body: &body,
            headers: BTreeMap::from([("content-type".to_owned(), "application/json".to_owned())]),
            now: Utc::now(),
        });

        let mut request = self.options.client.post(endpoint).body(body);
        for (name, value) in signed_headers {
            request = request.header(name, value);
        }

        let response = request.send().await.map_err(|error| {
            EmailSdkError::provider_error(error.to_string(), "ses")
                .retryable(error.is_timeout() || error.is_connect())
        })?;
        let status = response.status();
        let response_body = read_response_body(response).await.map_err(|error| {
            EmailSdkError::provider_error(error.to_string(), "ses").retryable(false)
        })?;

        if !status.is_success() {
            return Err(EmailSdkError::provider_error(
                http_error_message("ses", status.as_u16(), &response_body),
                "ses",
            )
            .status(status.as_u16())
            .retryable(is_retryable_status(status.as_u16()))
            .details(response_body.to_string()));
        }

        let parsed = serde_json::from_value::<SesResponse>(response_body.clone()).ok();
        let message_id = parsed.and_then(|body| body.message_id);

        Ok(EmailProviderResponse {
            id: message_id.clone(),
            provider: "ses".to_owned(),
            message_id,
            accepted: Vec::new(),
            rejected: Vec::new(),
            raw: Some(response_body.to_string()),
        })
    }
}

#[derive(Debug, Deserialize)]
struct SesResponse {
    #[serde(rename = "MessageId")]
    message_id: Option<String>,
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
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;

    #[tokio::test]
    async fn sends_signed_request_to_ses() {
        let server = TestServer::start("HTTP/1.1 200 OK", r#"{"MessageId":"ses_123"}"#).await;
        let provider = ses(SesProviderOptions::new("access", "secret", "us-east-1")
            .session_token("session")
            .base_url(server.base_url())
            .configuration_set_name("config"));
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let response = client.send(message, None).await.unwrap();

        let request = server.request().await;
        assert_eq!(response.message_id.as_deref(), Some("ses_123"));
        assert!(request.contains("POST /v2/email/outbound-emails HTTP/1.1"));
        assert!(request.contains("authorization: AWS4-HMAC-SHA256 Credential=access/"));
        assert!(request.contains("/us-east-1/ses/aws4_request"));
        assert!(request.contains("x-amz-content-sha256:"));
        assert!(request.contains("x-amz-date:"));
        assert!(request.contains("x-amz-security-token: session"));
        assert!(request.contains("\"FromEmailAddress\":\"from@example.com\""));
        assert!(request.contains("\"ConfigurationSetName\":\"config\""));
    }

    #[tokio::test]
    async fn maps_ses_error_response() {
        let server = TestServer::start(
            "HTTP/1.1 429 Too Many Requests",
            r#"{"message":"Slow down"}"#,
        )
        .await;
        let provider =
            ses(SesProviderOptions::new("access", "secret", "us-east-1")
                .base_url(server.base_url()));
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let error = client.send(message, None).await.unwrap_err();

        assert_eq!(error.provider.as_deref(), Some("ses"));
        assert_eq!(error.status, Some(429));
        assert!(error.retryable);
        assert_eq!(error.message, "ses failed with 429: Slow down");
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
                let mut buffer = vec![0; 16384];
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
