use std::collections::BTreeMap;

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Serialize;
use serde_json::Value;

use crate::{EmailAddress, EmailAttachment, EmailHeader, EmailMessage, Headers, MetadataValue};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MessageFieldSupport {
    pub cc: bool,
    pub bcc: bool,
    pub reply_to: bool,
    pub headers: bool,
    pub attachments: bool,
    pub tags: bool,
    pub metadata: bool,
}

impl MessageFieldSupport {
    pub const fn all() -> Self {
        Self {
            cc: true,
            bcc: true,
            reply_to: true,
            headers: true,
            attachments: true,
            tags: true,
            metadata: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ApiAddress {
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EmailParts {
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

pub fn email_parts(address: &EmailAddress) -> EmailParts {
    match address {
        EmailAddress::Named { email, name } => EmailParts {
            email: email.trim().to_owned(),
            name: Some(name.trim().trim_matches('"').to_owned()).filter(|value| !value.is_empty()),
        },
        EmailAddress::Address(address) => parse_email_parts(address),
    }
}

fn parse_email_parts(address: &str) -> EmailParts {
    let trimmed = address.trim();
    if let (Some(start), Some(end)) = (trimmed.rfind('<'), trimmed.rfind('>'))
        && start < end
    {
        let name = trimmed[..start].trim().trim_matches('"').trim();
        let email = trimmed[start + 1..end].trim();
        return EmailParts {
            email: email.to_owned(),
            name: Some(name.to_owned()).filter(|value| !value.is_empty()),
        };
    }

    EmailParts {
        email: trimmed.to_owned(),
        name: None,
    }
}

pub fn api_address(address: &EmailAddress) -> ApiAddress {
    match address {
        EmailAddress::Address(email) => ApiAddress {
            email: email.trim().to_owned(),
            name: None,
        },
        EmailAddress::Named { email, name } => ApiAddress {
            email: email.trim().to_owned(),
            name: Some(name.trim().to_owned()).filter(|value| !value.is_empty()),
        },
    }
}

pub fn api_addresses(addresses: &[EmailAddress]) -> Vec<ApiAddress> {
    addresses.iter().map(api_address).collect()
}

pub fn optional_api_addresses(addresses: &[EmailAddress]) -> Option<Vec<ApiAddress>> {
    if addresses.is_empty() {
        None
    } else {
        Some(api_addresses(addresses))
    }
}

pub fn optional_single_api_address(
    adapter: &str,
    field: &str,
    addresses: &[EmailAddress],
) -> Result<Option<ApiAddress>, crate::EmailSdkError> {
    if addresses.is_empty() {
        return Ok(None);
    }

    assert_max_items(adapter, field, addresses.len(), 1)?;
    Ok(addresses.first().map(api_address))
}

pub fn format_address(address: &EmailAddress) -> String {
    address.formatted()
}

pub fn format_addresses(addresses: &[EmailAddress]) -> Vec<String> {
    addresses.iter().map(format_address).collect()
}

pub fn headers_to_object(headers: &Option<Headers>) -> Option<BTreeMap<String, String>> {
    match headers {
        Some(Headers::Map(headers)) if !headers.is_empty() => Some(headers.clone()),
        Some(Headers::List(headers)) if !headers.is_empty() => Some(
            headers
                .iter()
                .map(|header| (header.name.clone(), header.value.clone()))
                .collect(),
        ),
        _ => None,
    }
}

pub fn headers_to_array(headers: &Option<Headers>) -> Option<Vec<EmailHeader>> {
    match headers {
        Some(Headers::List(headers)) if !headers.is_empty() => Some(headers.clone()),
        Some(Headers::Map(headers)) if !headers.is_empty() => Some(
            headers
                .iter()
                .map(|(name, value)| EmailHeader {
                    name: name.clone(),
                    value: value.clone(),
                })
                .collect(),
        ),
        _ => None,
    }
}

pub fn metadata_to_json_object(
    metadata: &BTreeMap<String, MetadataValue>,
) -> Option<serde_json::Map<String, Value>> {
    if metadata.is_empty() {
        return None;
    }

    Some(
        metadata
            .iter()
            .map(|(key, value)| {
                let value = match value {
                    MetadataValue::String(value) => Value::String(value.clone()),
                    MetadataValue::Number(value) => serde_json::Number::from_f64(*value)
                        .map(Value::Number)
                        .unwrap_or(Value::Null),
                    MetadataValue::Bool(value) => Value::Bool(*value),
                    MetadataValue::Null => Value::Null,
                };
                (key.clone(), value)
            })
            .collect(),
    )
}

pub fn is_retryable_status(status: u16) -> bool {
    matches!(status, 408 | 409 | 425 | 429) || status >= 500
}

pub fn http_error_message(provider: &str, status: u16, body: &Value) -> String {
    if let Some(message) = body_message(body) {
        return format!("{provider} failed with {status}: {message}");
    }

    format!("{provider} failed with HTTP {status}.")
}

pub async fn attachment_to_base64(
    attachment: &EmailAttachment,
) -> Result<String, crate::EmailSdkError> {
    Ok(STANDARD.encode(attachment_to_bytes(attachment).await?))
}

pub async fn attachment_to_bytes(
    attachment: &EmailAttachment,
) -> Result<Vec<u8>, crate::EmailSdkError> {
    if let Some(path) = &attachment.path {
        let bytes = tokio::fs::read(path).await.map_err(|error| {
            crate::EmailSdkError::validation(format!(
                "Attachment \"{}\" could not be read: {error}.",
                attachment.filename
            ))
        })?;
        return Ok(bytes);
    }

    Ok(attachment.content.clone().unwrap_or_default())
}

pub fn assert_supported_message_fields(
    adapter: &str,
    message: &EmailMessage,
    supported: MessageFieldSupport,
) -> Result<(), crate::EmailSdkError> {
    let mut unsupported = Vec::new();

    if !message.cc.is_empty() && !supported.cc {
        unsupported.push("cc");
    }
    if !message.bcc.is_empty() && !supported.bcc {
        unsupported.push("bcc");
    }
    if !message.reply_to.is_empty() && !supported.reply_to {
        unsupported.push("replyTo");
    }
    if headers_to_object(&message.headers).is_some() && !supported.headers {
        unsupported.push("headers");
    }
    if !message.attachments.is_empty() && !supported.attachments {
        unsupported.push("attachments");
    }
    if !message.tags.is_empty() && !supported.tags {
        unsupported.push("tags");
    }
    if !message.metadata.is_empty() && !supported.metadata {
        unsupported.push("metadata");
    }

    if unsupported.is_empty() {
        return Ok(());
    }

    Err(crate::EmailSdkError::validation(format!(
        "{adapter} does not support these EmailMessage fields: {}.",
        unsupported.join(", ")
    )))
}

pub fn assert_max_items(
    adapter: &str,
    field: &str,
    count: usize,
    max: usize,
) -> Result<(), crate::EmailSdkError> {
    if count <= max {
        return Ok(());
    }

    let suffix = if max == 1 { "" } else { "s" };
    Err(crate::EmailSdkError::validation(format!(
        "{adapter} only supports {max} {field}{suffix} per message."
    )))
}

fn body_message(body: &Value) -> Option<&str> {
    if let Value::Object(record) = body {
        for key in ["message", "Message", "error", "ErrorCode"] {
            if let Some(Value::String(message)) = record.get(key) {
                return Some(message);
            }
        }

        if let Some(Value::Array(errors)) = record.get("errors") {
            return errors.iter().find_map(|error| {
                error
                    .as_object()
                    .and_then(|record| record.get("message"))
                    .and_then(Value::as_str)
            });
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn formats_http_error_message_from_nested_errors() {
        let body = json!({ "errors": [{ "message": "Nope" }] });

        assert_eq!(
            http_error_message("Resend", 422, &body),
            "Resend failed with 422: Nope"
        );
    }

    #[test]
    fn detects_unsupported_fields() {
        let message = EmailMessage::builder("from@example.com", "to@example.com", "Hi")
            .text("Hi")
            .cc("copy@example.com")
            .build();

        let error =
            assert_supported_message_fields("tiny", &message, MessageFieldSupport::default())
                .unwrap_err();

        assert_eq!(error.code, "validation_error");
        assert!(error.message.contains("cc"));
    }
}
