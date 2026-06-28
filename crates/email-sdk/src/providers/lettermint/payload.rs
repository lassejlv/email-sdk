use std::collections::BTreeMap;

use email_sdk_core::{
    EmailMessage, EmailSdkError, MessageFieldSupport, MetadataValue, assert_max_items,
    assert_supported_message_fields, attachment_to_base64, format_address, format_addresses,
    headers_to_object,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct LettermintPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    route: Option<String>,
    from: String,
    to: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cc: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    bcc: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    reply_to: Vec<String>,
    subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    html: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    headers: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<LettermintAttachment>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct LettermintAttachment {
    filename: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_id: Option<String>,
}

pub(crate) async fn to_lettermint_payload(
    message: &EmailMessage,
    route: Option<&str>,
) -> Result<LettermintPayload, EmailSdkError> {
    assert_supported_message_fields(
        "lettermint",
        message,
        MessageFieldSupport {
            cc: true,
            bcc: true,
            reply_to: true,
            headers: true,
            attachments: true,
            tags: true,
            metadata: true,
        },
    )?;
    assert_max_items("lettermint", "tag", message.tags.len(), 1)?;

    let mut attachments = Vec::with_capacity(message.attachments.len());
    for attachment in &message.attachments {
        attachments.push(LettermintAttachment {
            filename: attachment.filename.clone(),
            content: attachment_to_base64(attachment).await?,
            content_type: attachment.content_type.clone(),
            content_id: attachment.content_id.clone(),
        });
    }

    Ok(LettermintPayload {
        route: route.map(ToOwned::to_owned),
        from: format_address(&message.from),
        to: format_addresses(&message.to),
        cc: format_addresses(&message.cc),
        bcc: format_addresses(&message.bcc),
        reply_to: format_addresses(&message.reply_to),
        subject: message.subject.clone(),
        html: message.html.clone(),
        text: message.text.clone(),
        tag: message.tags.first().map(|tag| tag.value.clone()),
        headers: headers_to_object(&message.headers),
        metadata: lettermint_metadata(&message.metadata),
        attachments,
    })
}

fn lettermint_metadata(
    metadata: &BTreeMap<String, MetadataValue>,
) -> Option<BTreeMap<String, String>> {
    if metadata.is_empty() {
        return None;
    }

    Some(
        metadata
            .iter()
            .map(|(key, value)| {
                let value = match value {
                    MetadataValue::String(value) => value.clone(),
                    MetadataValue::Number(value) => value.to_string(),
                    MetadataValue::Bool(value) => value.to_string(),
                    MetadataValue::Null => String::new(),
                };
                (key.clone(), value)
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use email_sdk_core::{EmailAddress, EmailAttachment, EmailMessage, MetadataValue};
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn builds_lettermint_payload() {
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
        .metadata("plan", MetadataValue::String("pro".to_owned()))
        .metadata("count", MetadataValue::Number(3.0))
        .tag("kind", "welcome")
        .build();
        message.attachments.push(EmailAttachment {
            filename: "hello.txt".to_owned(),
            content: Some(b"hello".to_vec()),
            path: None,
            content_type: Some("text/plain".to_owned()),
            content_id: Some("hello".to_owned()),
            disposition: None,
        });

        let payload = to_lettermint_payload(&message, Some("transactional"))
            .await
            .unwrap();
        let value = serde_json::to_value(payload).unwrap();

        assert_eq!(
            value,
            json!({
                "route": "transactional",
                "from": "Sender <sender@example.com>",
                "to": ["User <user@example.com>"],
                "cc": ["copy@example.com"],
                "reply_to": ["reply@example.com"],
                "subject": "Hello",
                "html": "<strong>Hello</strong>",
                "text": "Hello",
                "tag": "welcome",
                "headers": { "X-Test": "yes" },
                "metadata": { "count": "3", "plan": "pro" },
                "attachments": [{
                    "filename": "hello.txt",
                    "content": "aGVsbG8=",
                    "content_type": "text/plain",
                    "content_id": "hello"
                }]
            })
        );
    }

    #[tokio::test]
    async fn rejects_multiple_tags() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .tag("kind", "welcome")
            .tag("other", "extra")
            .build();

        let error = to_lettermint_payload(&message, None).await.unwrap_err();

        assert_eq!(error.message, "lettermint only supports 1 tag per message.");
    }
}
