use email_sdk_core::{
    EmailMessage, EmailSdkError, MessageFieldSupport, MetadataValue, assert_max_items,
    assert_supported_message_fields, attachment_to_base64, format_address, format_addresses,
    metadata_to_json_object,
};
use serde::Serialize;

const RESERVED_METADATA_KEYS: &[&str] = &[
    "sequenzySlug",
    "sequenzyPreview",
    "subscriberExternalId",
    "sequenzySubscriberExternalId",
];

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct SequenzyPayload {
    to: SequenzyRecipients,
    #[serde(skip_serializing_if = "Option::is_none")]
    slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    variables: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(
        rename = "subscriberExternalId",
        skip_serializing_if = "Option::is_none"
    )]
    subscriber_external_id: Option<String>,
    from: String,
    #[serde(rename = "replyTo", skip_serializing_if = "Option::is_none")]
    reply_to: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<SequenzyAttachment>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(untagged)]
enum SequenzyRecipients {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(untagged)]
enum SequenzyAttachment {
    Url { filename: String, path: String },
    Content { filename: String, content: String },
}

pub(crate) async fn to_sequenzy_payload(
    message: &EmailMessage,
) -> Result<SequenzyPayload, EmailSdkError> {
    assert_supported_message_fields(
        "sequenzy",
        message,
        MessageFieldSupport {
            cc: false,
            bcc: false,
            reply_to: true,
            headers: false,
            attachments: true,
            tags: false,
            metadata: true,
        },
    )?;

    let recipients = format_addresses(&message.to);
    let reply_to = format_addresses(&message.reply_to);
    assert_max_items("sequenzy", "recipient", recipients.len(), 50)?;
    assert_max_items("sequenzy", "replyTo", reply_to.len(), 1)?;

    let slug = string_metadata(message, "sequenzySlug");
    let preview = string_metadata(message, "sequenzyPreview");
    let subscriber_external_id = string_metadata(message, "subscriberExternalId")
        .or_else(|| string_metadata(message, "sequenzySubscriberExternalId"));
    let variables = sequenzy_variables(message);

    let mut attachments = Vec::with_capacity(message.attachments.len());
    for attachment in &message.attachments {
        if let Some(path) = &attachment.path
            && (path.starts_with("http://") || path.starts_with("https://"))
        {
            attachments.push(SequenzyAttachment::Url {
                filename: attachment.filename.clone(),
                path: path.clone(),
            });
            continue;
        }

        attachments.push(SequenzyAttachment::Content {
            filename: attachment.filename.clone(),
            content: attachment_to_base64(attachment).await?,
        });
    }

    Ok(SequenzyPayload {
        to: sequenzy_recipients(recipients),
        slug: slug.clone(),
        subject: if slug.is_some() {
            None
        } else {
            Some(message.subject.clone())
        },
        body: if slug.is_some() {
            None
        } else {
            message.html.clone().or_else(|| message.text.clone())
        },
        preview,
        variables,
        subscriber_external_id,
        from: format_address(&message.from),
        reply_to: reply_to.into_iter().next(),
        attachments,
    })
}

fn sequenzy_recipients(recipients: Vec<String>) -> SequenzyRecipients {
    if recipients.len() == 1 {
        SequenzyRecipients::One(recipients.into_iter().next().unwrap_or_default())
    } else {
        SequenzyRecipients::Many(recipients)
    }
}

fn string_metadata(message: &EmailMessage, key: &str) -> Option<String> {
    match message.metadata.get(key) {
        Some(MetadataValue::String(value)) if !value.is_empty() => Some(value.clone()),
        _ => None,
    }
}

fn sequenzy_variables(
    message: &EmailMessage,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    let mut variables = metadata_to_json_object(&message.metadata)?;
    for key in RESERVED_METADATA_KEYS {
        variables.remove(*key);
    }

    if variables.is_empty() {
        None
    } else {
        Some(variables)
    }
}

#[cfg(test)]
mod tests {
    use email_sdk_core::{EmailAttachment, EmailMessage, MetadataValue};
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn builds_sequenzy_payload_for_plain_message() {
        let mut message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .to("second@example.com")
            .html("<strong>Hello</strong>")
            .reply_to("reply@example.com")
            .metadata("plan", MetadataValue::String("pro".to_owned()))
            .metadata(
                "sequenzyPreview",
                MetadataValue::String("Preview text".to_owned()),
            )
            .metadata(
                "subscriberExternalId",
                MetadataValue::String("sub_123".to_owned()),
            )
            .build();
        message.attachments.push(EmailAttachment {
            filename: "remote.pdf".to_owned(),
            content: None,
            path: Some("https://example.com/remote.pdf".to_owned()),
            content_type: None,
            content_id: None,
            disposition: None,
        });
        message.attachments.push(EmailAttachment {
            filename: "hello.txt".to_owned(),
            content: Some(b"hello".to_vec()),
            path: None,
            content_type: Some("text/plain".to_owned()),
            content_id: None,
            disposition: None,
        });

        let payload = to_sequenzy_payload(&message).await.unwrap();
        let value = serde_json::to_value(payload).unwrap();

        assert_eq!(
            value,
            json!({
                "to": ["user@example.com", "second@example.com"],
                "subject": "Hello",
                "body": "<strong>Hello</strong>",
                "preview": "Preview text",
                "variables": { "plan": "pro" },
                "subscriberExternalId": "sub_123",
                "from": "sender@example.com",
                "replyTo": "reply@example.com",
                "attachments": [
                    { "filename": "remote.pdf", "path": "https://example.com/remote.pdf" },
                    { "filename": "hello.txt", "content": "aGVsbG8=" }
                ]
            })
        );
    }

    #[tokio::test]
    async fn builds_sequenzy_payload_for_slug_message() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .html("<strong>Hello</strong>")
            .metadata("sequenzySlug", MetadataValue::String("welcome".to_owned()))
            .metadata("name", MetadataValue::String("User".to_owned()))
            .build();

        let payload = to_sequenzy_payload(&message).await.unwrap();
        let value = serde_json::to_value(payload).unwrap();

        assert_eq!(
            value,
            json!({
                "to": "user@example.com",
                "slug": "welcome",
                "variables": { "name": "User" },
                "from": "sender@example.com"
            })
        );
    }

    #[tokio::test]
    async fn rejects_cc() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .cc("copy@example.com")
            .build();

        let error = to_sequenzy_payload(&message).await.unwrap_err();

        assert_eq!(error.code, "validation_error");
        assert!(error.message.contains("cc"));
    }

    #[tokio::test]
    async fn rejects_multiple_reply_to_addresses() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .reply_to("reply@example.com")
            .reply_to("other@example.com")
            .build();

        let error = to_sequenzy_payload(&message).await.unwrap_err();

        assert_eq!(
            error.message,
            "sequenzy only supports 1 replyTo per message."
        );
    }
}
