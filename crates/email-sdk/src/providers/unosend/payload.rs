use std::collections::BTreeMap;

use email_sdk_core::{
    EmailMessage, EmailSdkError, MessageFieldSupport, assert_supported_message_fields,
    attachment_to_base64, format_address, format_addresses, headers_to_object,
    optional_single_api_address,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct UnosendPayload {
    from: String,
    to: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cc: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bcc: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_to: Option<String>,
    subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    html: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    headers: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<UnosendAttachment>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct UnosendAttachment {
    filename: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_type: Option<String>,
}

pub(crate) async fn to_unosend_payload(
    message: &EmailMessage,
) -> Result<UnosendPayload, EmailSdkError> {
    assert_supported_message_fields(
        "unosend",
        message,
        MessageFieldSupport {
            cc: true,
            bcc: true,
            reply_to: true,
            headers: true,
            attachments: true,
            tags: true,
            metadata: false,
        },
    )?;
    optional_single_api_address("unosend", "replyTo", &message.reply_to)?;

    let mut attachments = Vec::with_capacity(message.attachments.len());
    for attachment in &message.attachments {
        attachments.push(UnosendAttachment {
            filename: attachment.filename.clone(),
            content: attachment_to_base64(attachment).await?,
            content_type: attachment.content_type.clone(),
        });
    }

    Ok(UnosendPayload {
        from: format_address(&message.from),
        to: format_addresses(&message.to),
        cc: optional_string_addresses(&message.cc),
        bcc: optional_string_addresses(&message.bcc),
        reply_to: format_addresses(&message.reply_to).into_iter().next(),
        subject: message.subject.clone(),
        html: message.html.clone(),
        text: message.text.clone(),
        headers: headers_to_object(&message.headers),
        tags: message.tags.iter().map(|tag| tag.value.clone()).collect(),
        attachments,
    })
}

fn optional_string_addresses(addresses: &[email_sdk_core::EmailAddress]) -> Option<Vec<String>> {
    if addresses.is_empty() {
        None
    } else {
        Some(format_addresses(addresses))
    }
}

#[cfg(test)]
mod tests {
    use email_sdk_core::{
        EmailAddress, EmailAttachment, EmailAttachmentDisposition, EmailMessage, MetadataValue,
    };
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn builds_unosend_payload() {
        let mut message = EmailMessage::builder(
            EmailAddress::named("sender@example.com", "Sender"),
            EmailAddress::named("user@example.com", "User"),
            "Hello",
        )
        .text("Hello")
        .html("<strong>Hello</strong>")
        .cc("copy@example.com")
        .reply_to("reply@example.com")
        .header("X-Test", "yes")
        .tag("kind", "welcome")
        .build();
        message.attachments.push(EmailAttachment {
            filename: "hello.txt".to_owned(),
            content: Some(b"hello".to_vec()),
            path: None,
            content_type: Some("text/plain".to_owned()),
            content_id: Some("hello".to_owned()),
            disposition: Some(EmailAttachmentDisposition::Inline),
        });

        let payload = to_unosend_payload(&message).await.unwrap();
        let value = serde_json::to_value(payload).unwrap();

        assert_eq!(
            value,
            json!({
                "from": "Sender <sender@example.com>",
                "to": ["User <user@example.com>"],
                "cc": ["copy@example.com"],
                "reply_to": "reply@example.com",
                "subject": "Hello",
                "html": "<strong>Hello</strong>",
                "text": "Hello",
                "headers": { "X-Test": "yes" },
                "tags": ["welcome"],
                "attachments": [{
                    "filename": "hello.txt",
                    "content": "aGVsbG8=",
                    "content_type": "text/plain"
                }]
            })
        );
    }

    #[tokio::test]
    async fn rejects_metadata() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .metadata("plan", MetadataValue::String("pro".to_owned()))
            .build();

        let error = to_unosend_payload(&message).await.unwrap_err();

        assert_eq!(error.code, "validation_error");
        assert!(error.message.contains("metadata"));
    }

    #[tokio::test]
    async fn rejects_multiple_reply_to_addresses() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .reply_to("reply@example.com")
            .reply_to("other@example.com")
            .build();

        let error = to_unosend_payload(&message).await.unwrap_err();

        assert_eq!(
            error.message,
            "unosend only supports 1 replyTo per message."
        );
    }
}
