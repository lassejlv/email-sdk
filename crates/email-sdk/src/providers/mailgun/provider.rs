use std::sync::Arc;

use async_trait::async_trait;
use email_sdk_core::{
    EmailMessage, EmailProvider, EmailProviderContext, EmailProviderResponse, EmailSdkError,
    SharedEmailProvider, http_error_message, is_retryable_status,
};

use super::payload::to_mailgun_form;

#[derive(Debug, Clone)]
pub struct MailgunProviderOptions {
    pub api_key: String,
    pub domain: String,
    pub base_url: String,
    pub client: reqwest::Client,
}

impl MailgunProviderOptions {
    pub fn new(api_key: impl Into<String>, domain: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            domain: domain.into(),
            base_url: "https://api.mailgun.net".to_owned(),
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
pub struct MailgunProvider {
    options: MailgunProviderOptions,
}

impl MailgunProvider {
    pub fn new(options: MailgunProviderOptions) -> Self {
        Self { options }
    }

    pub fn shared(options: MailgunProviderOptions) -> SharedEmailProvider {
        Arc::new(Self::new(options))
    }

    pub fn base_url(&self) -> &str {
        &self.options.base_url
    }
}

pub fn mailgun(options: MailgunProviderOptions) -> Arc<MailgunProvider> {
    Arc::new(MailgunProvider::new(options))
}

#[async_trait]
impl EmailProvider for MailgunProvider {
    fn name(&self) -> &str {
        "mailgun"
    }

    async fn send(
        &self,
        message: EmailMessage,
        _context: EmailProviderContext,
    ) -> Result<EmailProviderResponse, EmailSdkError> {
        let form = to_mailgun_form(&message).await?;
        let url = format!(
            "{}/v3/{}/messages",
            self.options.base_url.trim_end_matches('/'),
            self.options.domain
        );
        let response = self
            .options
            .client
            .post(url)
            .basic_auth("api", Some(&self.options.api_key))
            .multipart(form)
            .send()
            .await
            .map_err(|error| {
                EmailSdkError::provider_error(error.to_string(), "mailgun")
                    .retryable(error.is_timeout() || error.is_connect())
            })?;
        let status = response.status();
        let body = read_response_body(response).await.map_err(|error| {
            EmailSdkError::provider_error(error.to_string(), "mailgun").retryable(false)
        })?;

        if !status.is_success() {
            return Err(EmailSdkError::provider_error(
                http_error_message("mailgun", status.as_u16(), &body),
                "mailgun",
            )
            .status(status.as_u16())
            .retryable(is_retryable_status(status.as_u16()))
            .details(body.to_string()));
        }

        let id = body
            .as_object()
            .and_then(|record| record.get("id"))
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned);

        Ok(EmailProviderResponse {
            id: id.clone(),
            provider: "mailgun".to_owned(),
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

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use email_sdk_core::{
        EmailAttachment, EmailAttachmentDisposition, EmailClientOptions, EmailMessage,
        MetadataValue, create_email_client,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;

    #[tokio::test]
    async fn sends_multipart_request_to_mailgun() {
        let server = TestServer::start(
            "HTTP/1.1 200 OK",
            r#"{"id":"mg_123","message":"Queued. Thank you."}"#,
        )
        .await;
        let provider = mailgun(
            MailgunProviderOptions::new("secret", "example.com").base_url(server.base_url()),
        );
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let mut message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .html("<strong>Hi</strong>")
            .cc("copy@example.com")
            .reply_to("reply@example.com")
            .header("X-Test", "yes")
            .metadata("plan", MetadataValue::String("pro".to_owned()))
            .tag("kind", "welcome")
            .build();
        message.attachments.push(EmailAttachment {
            filename: "hello.txt".to_owned(),
            content: Some(b"hello".to_vec()),
            path: None,
            content_type: Some("text/plain".to_owned()),
            content_id: None,
            disposition: Some(EmailAttachmentDisposition::Inline),
        });

        let response = client.send(message, None).await.unwrap();

        let request = server.request().await;
        assert_eq!(response.message_id.as_deref(), Some("mg_123"));
        assert!(request.contains("POST /v3/example.com/messages HTTP/1.1"));
        assert!(request.contains("authorization: Basic YXBpOnNlY3JldA=="));
        assert!(request.contains("multipart/form-data; boundary="));
        assert!(request.contains("name=\"from\""));
        assert!(request.contains("from@example.com"));
        assert!(request.contains("name=\"to\""));
        assert!(request.contains("to@example.com"));
        assert!(request.contains("name=\"h:Reply-To\""));
        assert!(request.contains("reply@example.com"));
        assert!(request.contains("name=\"h:X-Test\""));
        assert!(request.contains("name=\"v:plan\""));
        assert!(request.contains("name=\"o:tag\""));
        assert!(request.contains("name=\"inline\"; filename=\"hello.txt\""));
        assert!(request.contains("hello"));
    }

    #[tokio::test]
    async fn maps_mailgun_error_response() {
        let server = TestServer::start(
            "HTTP/1.1 500 Internal Server Error",
            r#"{"message":"Down"}"#,
        )
        .await;
        let provider = mailgun(
            MailgunProviderOptions::new("secret", "example.com").base_url(server.base_url()),
        );
        let client = create_email_client(EmailClientOptions::new().adapter(provider)).unwrap();
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hi")
            .build();

        let error = client.send(message, None).await.unwrap_err();

        assert_eq!(error.provider.as_deref(), Some("mailgun"));
        assert_eq!(error.status, Some(500));
        assert!(error.retryable);
        assert_eq!(error.message, "mailgun failed with 500: Down");
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
                let mut read = stream.read(&mut buffer).await.unwrap();
                let mut raw_request = String::from_utf8_lossy(&buffer[..read]).to_string();
                if let Some(content_length) = content_length(&raw_request) {
                    while body_len(&raw_request) < content_length {
                        let next = stream.read(&mut buffer[read..]).await.unwrap();
                        if next == 0 {
                            break;
                        }
                        read += next;
                        raw_request = String::from_utf8_lossy(&buffer[..read]).to_string();
                    }
                }
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

    fn content_length(request: &str) -> Option<usize> {
        request.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())
                .flatten()
        })
    }

    fn body_len(request: &str) -> usize {
        request
            .split_once("\r\n\r\n")
            .map(|(_, body)| body.len())
            .unwrap_or_default()
    }
}
