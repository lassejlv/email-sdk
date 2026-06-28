use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use email_sdk_core::{
    EmailMessage, EmailProvider, EmailProviderContext, EmailProviderResponse, EmailSdkError,
    MessageFieldSupport, SharedEmailProvider, assert_supported_message_fields, format_address,
    format_addresses, headers_to_array, headers_to_object,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};
use tokio_native_tls::{TlsConnector, TlsStream, native_tls};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpAuthMethod {
    Plain,
    Login,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtpAuth {
    pub user: String,
    pub pass: String,
    pub method: SmtpAuthMethod,
}

impl SmtpAuth {
    pub fn new(user: impl Into<String>, pass: impl Into<String>) -> Self {
        Self {
            user: user.into(),
            pass: pass.into(),
            method: SmtpAuthMethod::Plain,
        }
    }

    pub fn method(mut self, method: SmtpAuthMethod) -> Self {
        self.method = method;
        self
    }
}

#[derive(Debug, Clone)]
pub struct SmtpProviderOptions {
    pub host: String,
    pub port: Option<u16>,
    pub secure: bool,
    pub auth: Option<SmtpAuth>,
    pub default_reply_to: Option<String>,
    pub require_tls: bool,
    pub allow_insecure_auth: bool,
    pub name: String,
    pub helo_name: String,
    pub timeout: Duration,
}

impl SmtpProviderOptions {
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: None,
            secure: false,
            auth: None,
            default_reply_to: None,
            require_tls: false,
            allow_insecure_auth: false,
            name: "smtp".to_owned(),
            helo_name: "localhost".to_owned(),
            timeout: Duration::from_secs(15),
        }
    }

    pub fn port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    pub fn secure(mut self, secure: bool) -> Self {
        self.secure = secure;
        self
    }

    pub fn auth(mut self, auth: SmtpAuth) -> Self {
        self.auth = Some(auth);
        self
    }

    pub fn default_reply_to(mut self, reply_to: impl Into<String>) -> Self {
        self.default_reply_to = Some(reply_to.into());
        self
    }

    pub fn require_tls(mut self, require_tls: bool) -> Self {
        self.require_tls = require_tls;
        self
    }

    pub fn allow_insecure_auth(mut self, allow: bool) -> Self {
        self.allow_insecure_auth = allow;
        self
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn helo_name(mut self, helo_name: impl Into<String>) -> Self {
        self.helo_name = helo_name.into();
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn port_or_default(&self) -> u16 {
        self.port.unwrap_or(if self.secure { 465 } else { 587 })
    }
}

#[derive(Debug, Clone)]
pub struct SmtpProvider {
    options: SmtpProviderOptions,
}

impl SmtpProvider {
    pub fn new(options: SmtpProviderOptions) -> Self {
        Self { options }
    }

    pub fn shared(options: SmtpProviderOptions) -> SharedEmailProvider {
        Arc::new(Self::new(options))
    }

    pub fn host(&self) -> &str {
        &self.options.host
    }

    pub fn port(&self) -> u16 {
        self.options.port_or_default()
    }
}

pub fn smtp(options: SmtpProviderOptions) -> Arc<SmtpProvider> {
    Arc::new(SmtpProvider::new(options))
}

#[async_trait]
impl EmailProvider for SmtpProvider {
    fn name(&self) -> &str {
        &self.options.name
    }

    async fn send(
        &self,
        message: EmailMessage,
        _context: EmailProviderContext,
    ) -> Result<EmailProviderResponse, EmailSdkError> {
        assert_supported_message_fields(
            &self.options.name,
            &message,
            MessageFieldSupport {
                cc: true,
                bcc: true,
                reply_to: true,
                headers: true,
                attachments: false,
                tags: false,
                metadata: false,
            },
        )?;
        assert_smtp_message(&self.options.name, &message)?;

        let mut client = SmtpClient::connect(self.options.clone())
            .await
            .map_err(|error| smtp_error(error, &self.options.name))?;
        let response = client
            .send(&message)
            .await
            .map_err(|error| smtp_error(error, &self.options.name))?;

        Ok(EmailProviderResponse {
            id: response.message_id.clone(),
            provider: self.options.name.clone(),
            message_id: response.message_id,
            accepted: response.accepted,
            rejected: Vec::new(),
            raw: Some(response.response),
        })
    }
}

fn smtp_error(error: EmailSdkError, provider: &str) -> EmailSdkError {
    EmailSdkError::provider_error(error.message, provider).retryable(true)
}

struct SmtpSendResponse {
    message_id: Option<String>,
    accepted: Vec<String>,
    response: String,
}

struct SmtpClient {
    options: SmtpProviderOptions,
    stream: SmtpConnection,
    buffer: Vec<u8>,
    tls_active: bool,
}

enum SmtpConnection {
    Plain(Option<TcpStream>),
    Tls(TlsStream<TcpStream>),
}

impl SmtpClient {
    async fn connect(options: SmtpProviderOptions) -> Result<Self, EmailSdkError> {
        let address = format!("{}:{}", options.host, options.port_or_default());
        let tcp = timeout(options.timeout, TcpStream::connect(address))
            .await
            .map_err(|_| EmailSdkError::validation("SMTP connection timed out."))?
            .map_err(|error| EmailSdkError::validation(error.to_string()))?;

        let (stream, tls_active) = if options.secure {
            let connector = tls_connector()?;
            let tls = timeout(options.timeout, connector.connect(&options.host, tcp))
                .await
                .map_err(|_| EmailSdkError::validation("SMTP TLS connection timed out."))?
                .map_err(|error| EmailSdkError::validation(error.to_string()))?;
            (SmtpConnection::Tls(tls), true)
        } else {
            (SmtpConnection::Plain(Some(tcp)), false)
        };

        Ok(Self {
            options,
            stream,
            buffer: Vec::new(),
            tls_active,
        })
    }

    async fn send(&mut self, message: &EmailMessage) -> Result<SmtpSendResponse, EmailSdkError> {
        self.expect(&[220]).await?;
        self.command(&format!("EHLO {}", self.options.helo_name), &[250])
            .await?;

        let should_start_tls = !self.tls_active
            && (self.options.require_tls
                || (self.options.auth.is_some() && !self.options.allow_insecure_auth));

        if should_start_tls {
            self.command("STARTTLS", &[220]).await?;
            self.upgrade_to_tls().await?;
            self.command(&format!("EHLO {}", self.options.helo_name), &[250])
                .await?;
        }

        if self.options.auth.is_some()
            && !self.tls_active
            && !should_start_tls
            && !self.options.allow_insecure_auth
        {
            return Err(EmailSdkError::validation(
                "SMTP auth requires TLS. Set secure, requireTLS, or allowInsecureAuth.",
            ));
        }

        if self.options.auth.is_some() {
            self.authenticate().await?;
        }

        let from = envelope_address(&message.from);
        let recipients = [
            format_addresses(&message.to),
            format_addresses(&message.cc),
            format_addresses(&message.bcc),
        ]
        .concat()
        .into_iter()
        .map(|address| parse_email_address(&address))
        .collect::<Vec<_>>();

        self.command(&format!("MAIL FROM:<{from}>"), &[250]).await?;

        let mut accepted = Vec::new();
        for recipient in recipients {
            self.command(&format!("RCPT TO:<{recipient}>"), &[250, 251])
                .await?;
            accepted.push(recipient);
        }

        self.command("DATA", &[354]).await?;
        let raw = build_mime_message(message, self.options.default_reply_to.as_deref());
        let response = self
            .command(&format!("{}\r\n.", escape_data(&raw)), &[250])
            .await?;
        let _ = self.command("QUIT", &[221]).await;

        Ok(SmtpSendResponse {
            message_id: extract_smtp_message_id(&response)
                .or_else(|| message.idempotency_key.clone()),
            accepted,
            response,
        })
    }

    async fn upgrade_to_tls(&mut self) -> Result<(), EmailSdkError> {
        let connector = tls_connector()?;
        let tcp = match &mut self.stream {
            SmtpConnection::Plain(stream) => stream
                .take()
                .ok_or_else(|| EmailSdkError::validation("SMTP socket is not connected."))?,
            SmtpConnection::Tls(_) => {
                return Err(EmailSdkError::validation(
                    "SMTP socket is already using TLS.",
                ));
            }
        };
        let tls = timeout(
            self.options.timeout,
            connector.connect(&self.options.host, tcp),
        )
        .await
        .map_err(|_| EmailSdkError::validation("SMTP TLS upgrade timed out."))?
        .map_err(|error| EmailSdkError::validation(error.to_string()))?;
        self.stream = SmtpConnection::Tls(tls);
        self.tls_active = true;
        self.buffer.clear();
        Ok(())
    }

    async fn authenticate(&mut self) -> Result<(), EmailSdkError> {
        let auth = self.options.auth.clone().expect("checked auth exists");
        match auth.method {
            SmtpAuthMethod::Login => {
                self.command("AUTH LOGIN", &[334]).await?;
                self.command(&STANDARD.encode(auth.user), &[334]).await?;
                self.command(&STANDARD.encode(auth.pass), &[235]).await?;
            }
            SmtpAuthMethod::Plain => {
                let payload = STANDARD.encode(format!("\0{}\0{}", auth.user, auth.pass));
                self.command(&format!("AUTH PLAIN {payload}"), &[235])
                    .await?;
            }
        }
        Ok(())
    }

    async fn command(&mut self, command: &str, expected: &[u16]) -> Result<String, EmailSdkError> {
        self.write_line(command).await?;
        self.expect(expected).await
    }

    async fn write_line(&mut self, value: &str) -> Result<(), EmailSdkError> {
        let data = format!("{value}\r\n");
        timeout(self.options.timeout, self.stream.write_all(data.as_bytes()))
            .await
            .map_err(|_| EmailSdkError::validation("SMTP command timed out."))?
            .map_err(|error| EmailSdkError::validation(error.to_string()))
    }

    async fn expect(&mut self, expected: &[u16]) -> Result<String, EmailSdkError> {
        loop {
            let line = self.read_line().await?;
            if line.len() < 3 {
                continue;
            }
            let code = line[..3].parse::<u16>().unwrap_or(0);
            if line.as_bytes().get(3) == Some(&b'-') {
                continue;
            }
            if expected.contains(&code) {
                return Ok(line);
            }
            return Err(EmailSdkError::validation(format!(
                "SMTP expected {} but received: {line}",
                expected
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join("/")
            )));
        }
    }

    async fn read_line(&mut self) -> Result<String, EmailSdkError> {
        loop {
            if let Some(index) = self.buffer.iter().position(|byte| *byte == b'\n') {
                let mut line = self.buffer.drain(..=index).collect::<Vec<_>>();
                while matches!(line.last(), Some(b'\n' | b'\r')) {
                    line.pop();
                }
                return String::from_utf8(line)
                    .map_err(|error| EmailSdkError::validation(error.to_string()));
            }

            let mut chunk = [0; 1024];
            let read = timeout(self.options.timeout, self.stream.read(&mut chunk))
                .await
                .map_err(|_| EmailSdkError::validation("SMTP command timed out."))?
                .map_err(|error| EmailSdkError::validation(error.to_string()))?;
            if read == 0 {
                return Err(EmailSdkError::validation("SMTP connection closed."));
            }
            self.buffer.extend_from_slice(&chunk[..read]);
        }
    }
}

impl SmtpConnection {
    async fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        match self {
            Self::Plain(Some(stream)) => stream.write_all(bytes).await,
            Self::Plain(None) => Err(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "SMTP socket is not connected.",
            )),
            Self::Tls(stream) => stream.write_all(bytes).await,
        }
    }

    async fn read(&mut self, chunk: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(Some(stream)) => stream.read(chunk).await,
            Self::Plain(None) => Err(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "SMTP socket is not connected.",
            )),
            Self::Tls(stream) => stream.read(chunk).await,
        }
    }
}

fn tls_connector() -> Result<TlsConnector, EmailSdkError> {
    native_tls::TlsConnector::builder()
        .build()
        .map(TlsConnector::from)
        .map_err(|error| EmailSdkError::validation(error.to_string()))
}

fn build_mime_message(message: &EmailMessage, default_reply_to: Option<&str>) -> String {
    let mut headers = vec![
        ("From".to_owned(), format_address(&message.from)),
        ("To".to_owned(), format_addresses(&message.to).join(", ")),
        ("Subject".to_owned(), message.subject.clone()),
        (
            "Date".to_owned(),
            "Thu, 01 Jan 1970 00:00:00 GMT".to_owned(),
        ),
        (
            "Message-ID".to_owned(),
            format!(
                "<{}@email-sdk.local>",
                message
                    .idempotency_key
                    .clone()
                    .unwrap_or_else(|| "generated".to_owned())
            ),
        ),
        ("MIME-Version".to_owned(), "1.0".to_owned()),
    ];

    if let Some(custom) = headers_to_object(&message.headers) {
        headers.extend(custom);
    }
    if !message.cc.is_empty() {
        headers.push(("Cc".to_owned(), format_addresses(&message.cc).join(", ")));
    }
    if !message.reply_to.is_empty() || default_reply_to.is_some() {
        headers.push((
            "Reply-To".to_owned(),
            if message.reply_to.is_empty() {
                default_reply_to.unwrap_or_default().to_owned()
            } else {
                format_addresses(&message.reply_to).join(", ")
            },
        ));
    }

    let header_text = headers
        .into_iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(key, value)| format!("{key}: {}", fold_header(&value)))
        .collect::<Vec<_>>()
        .join("\r\n");

    if let (Some(text), Some(html)) = (&message.text, &message.html) {
        let boundary = "email-sdk-boundary";
        return format!(
            "{header_text}\r\nContent-Type: multipart/alternative; boundary=\"{boundary}\"\r\n\r\n--{boundary}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{text}\r\n--{boundary}\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html}\r\n--{boundary}--"
        );
    }

    let content_type = if message.html.is_some() {
        "text/html"
    } else {
        "text/plain"
    };
    let body = message
        .html
        .as_deref()
        .or(message.text.as_deref())
        .unwrap_or("");
    format!("{header_text}\r\nContent-Type: {content_type}; charset=utf-8\r\n\r\n{body}")
}

fn assert_smtp_message(provider: &str, message: &EmailMessage) -> Result<(), EmailSdkError> {
    let addresses = [
        vec![format_address(&message.from)],
        format_addresses(&message.to),
        format_addresses(&message.cc),
        format_addresses(&message.bcc),
    ]
    .concat();

    for address in addresses {
        let envelope = parse_email_address(&address);
        if envelope.is_empty() || envelope.chars().any(is_forbidden_envelope_char) {
            return Err(EmailSdkError::validation(format!(
                "SMTP envelope address {envelope:?} contains invalid characters."
            ))
            .provider(provider));
        }
    }

    for header in headers_to_array(&message.headers).unwrap_or_default() {
        if !is_valid_header_name(&header.name) {
            return Err(EmailSdkError::validation(format!(
                "SMTP header name {:?} contains invalid characters.",
                header.name
            ))
            .provider(provider));
        }
    }

    Ok(())
}

fn envelope_address(address: &email_sdk_core::EmailAddress) -> String {
    parse_email_address(&format_address(address))
}

fn parse_email_address(address: &str) -> String {
    if let (Some(start), Some(end)) = (address.rfind('<'), address.rfind('>'))
        && start < end
    {
        return address[start + 1..end].trim().to_owned();
    }

    address.trim().to_owned()
}

fn is_forbidden_envelope_char(value: char) -> bool {
    value.is_whitespace() || value == '<' || value == '>' || value.is_control() || !value.is_ascii()
}

fn is_valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| matches!(byte, 0x21..=0x39 | 0x3b..=0x7e))
}

fn escape_data(value: &str) -> String {
    value
        .lines()
        .map(|line| {
            if line.starts_with('.') {
                format!(".{line}")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\r\n")
}

fn fold_header(value: &str) -> String {
    value.replace("\r\n", " ").replace(['\r', '\n'], " ")
}

fn extract_smtp_message_id(response: &str) -> Option<String> {
    let lower = response.to_lowercase();
    for marker in ["queued as", "id"] {
        if let Some(index) = lower.find(marker) {
            let value = response[index + marker.len()..]
                .trim()
                .trim_start_matches('<')
                .split(['>', ' '])
                .next()
                .unwrap_or_default();
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use email_sdk_core::{EmailMessage, MetadataValue};
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
        net::TcpListener,
    };

    use super::*;

    #[test]
    fn builds_multipart_mime_message() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .html("<strong>Hello</strong>")
            .cc("copy@example.com")
            .reply_to("reply@example.com")
            .header("X-Test", "one\r\ntwo")
            .idempotency_key("idem_123")
            .build();

        let raw = build_mime_message(&message, None);

        assert!(raw.contains("From: sender@example.com"));
        assert!(raw.contains("To: user@example.com"));
        assert!(raw.contains("Cc: copy@example.com"));
        assert!(raw.contains("Reply-To: reply@example.com"));
        assert!(raw.contains("X-Test: one two"));
        assert!(raw.contains("Message-ID: <idem_123@email-sdk.local>"));
        assert!(raw.contains("multipart/alternative"));
    }

    #[test]
    fn validates_envelope_addresses() {
        let message = EmailMessage::builder("bad\r\n@example.com", "user@example.com", "Hello")
            .text("Hello")
            .build();

        let error = assert_smtp_message("smtp", &message).unwrap_err();

        assert!(error.message.contains("invalid characters"));
    }

    #[test]
    fn validates_header_names() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .header("Bad:Name", "value")
            .build();

        let error = assert_smtp_message("smtp", &message).unwrap_err();

        assert!(error.message.contains("SMTP header name"));
    }

    #[tokio::test]
    async fn rejects_attachments() {
        let mut message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .metadata("plan", MetadataValue::String("pro".to_owned()))
            .build();
        message.attachments.push(email_sdk_core::EmailAttachment {
            filename: "hello.txt".to_owned(),
            content: Some(b"hello".to_vec()),
            path: None,
            content_type: None,
            content_id: None,
            disposition: None,
        });

        let provider = smtp(SmtpProviderOptions::new("localhost").allow_insecure_auth(true));
        let error = provider
            .send(message, EmailProviderContext::default())
            .await
            .unwrap_err();

        assert!(error.message.contains("attachments"));
        assert!(error.message.contains("metadata"));
    }

    #[tokio::test]
    async fn sends_plain_smtp_message() {
        let server = SmtpTestServer::start().await;
        let provider = smtp(
            SmtpProviderOptions::new("127.0.0.1")
                .port(server.address.port())
                .allow_insecure_auth(true),
        );
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .idempotency_key("idem_123")
            .build();

        let response = provider
            .send(message, EmailProviderContext::default())
            .await
            .unwrap();

        let transcript = server.transcript().await;
        assert_eq!(response.message_id.as_deref(), Some("queued_123"));
        assert_eq!(response.accepted, vec!["user@example.com"]);
        assert!(transcript.contains("EHLO localhost"));
        assert!(transcript.contains("MAIL FROM:<sender@example.com>"));
        assert!(transcript.contains("RCPT TO:<user@example.com>"));
        assert!(transcript.contains("Message-ID: <idem_123@email-sdk.local>"));
        assert!(transcript.contains("Hello"));
    }

    struct SmtpTestServer {
        address: SocketAddr,
        transcript: tokio::sync::oneshot::Receiver<String>,
    }

    impl SmtpTestServer {
        async fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let (send_transcript, transcript) = tokio::sync::oneshot::channel();

            tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let (read, mut write) = stream.into_split();
                let mut reader = BufReader::new(read);
                let mut transcript = String::new();

                write.write_all(b"220 localhost ready\r\n").await.unwrap();

                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).await.unwrap() == 0 {
                        break;
                    }
                    let command = line.trim_end_matches(['\r', '\n']).to_owned();
                    transcript.push_str(&command);
                    transcript.push('\n');

                    if command.starts_with("EHLO ") {
                        write.write_all(b"250 localhost\r\n").await.unwrap();
                    } else if command.starts_with("MAIL FROM:") || command.starts_with("RCPT TO:") {
                        write.write_all(b"250 ok\r\n").await.unwrap();
                    } else if command == "DATA" {
                        write.write_all(b"354 send data\r\n").await.unwrap();
                        loop {
                            let mut data_line = String::new();
                            reader.read_line(&mut data_line).await.unwrap();
                            let data = data_line.trim_end_matches(['\r', '\n']).to_owned();
                            if data == "." {
                                break;
                            }
                            transcript.push_str(&data);
                            transcript.push('\n');
                        }
                        write
                            .write_all(b"250 queued as queued_123\r\n")
                            .await
                            .unwrap();
                    } else if command == "QUIT" {
                        write.write_all(b"221 bye\r\n").await.unwrap();
                        break;
                    } else {
                        write.write_all(b"250 ok\r\n").await.unwrap();
                    }
                }

                let _ = send_transcript.send(transcript);
            });

            Self {
                address,
                transcript,
            }
        }

        async fn transcript(self) -> String {
            self.transcript.await.unwrap()
        }
    }
}
