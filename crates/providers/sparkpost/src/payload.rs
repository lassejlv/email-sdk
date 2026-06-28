use std::collections::BTreeMap;

use email_sdk_core::{
    ApiAddress, EmailMessage, EmailSdkError, MessageFieldSupport, api_address,
    assert_supported_message_fields, attachment_to_base64, format_addresses, headers_to_object,
    metadata_to_json_object,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct SparkPostPayload {
    options: SparkPostOptions,
    recipients: Vec<SparkPostRecipient>,
    content: SparkPostContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    substitution_data: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct SparkPostOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    sandbox: Option<bool>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct SparkPostRecipient {
    address: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct SparkPostContent {
    from: ApiAddress,
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
    attachments: Vec<SparkPostAttachment>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct SparkPostAttachment {
    name: String,
    #[serde(rename = "type")]
    content_type: String,
    data: String,
}

pub(crate) async fn to_sparkpost_payload(
    message: &EmailMessage,
    sandbox: Option<bool>,
) -> Result<SparkPostPayload, EmailSdkError> {
    assert_supported_message_fields(
        "sparkpost",
        message,
        MessageFieldSupport {
            cc: false,
            bcc: false,
            reply_to: true,
            headers: true,
            attachments: true,
            tags: true,
            metadata: true,
        },
    )?;

    let mut attachments = Vec::with_capacity(message.attachments.len());
    for attachment in &message.attachments {
        attachments.push(SparkPostAttachment {
            name: attachment.filename.clone(),
            content_type: attachment
                .content_type
                .clone()
                .unwrap_or_else(|| "application/octet-stream".to_owned()),
            data: attachment_to_base64(attachment).await?,
        });
    }

    Ok(SparkPostPayload {
        options: SparkPostOptions { sandbox },
        recipients: format_addresses(&message.to)
            .into_iter()
            .map(|address| SparkPostRecipient { address })
            .collect(),
        content: SparkPostContent {
            from: api_address(&message.from),
            reply_to: optional_string_addresses(&message.reply_to).map(|items| items.join(", ")),
            subject: message.subject.clone(),
            html: message.html.clone(),
            text: message.text.clone(),
            headers: headers_to_object(&message.headers),
            attachments,
        },
        metadata: metadata_to_json_object(&message.metadata),
        substitution_data: message
            .tags
            .iter()
            .map(|tag| (tag.name.clone(), tag.value.clone()))
            .collect(),
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
    use email_sdk_core::{EmailAddress, EmailAttachment, EmailMessage, EmailTag, MetadataValue};
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn builds_sparkpost_payload() {
        let mut message = EmailMessage::builder(
            EmailAddress::named("sender@example.com", "Sender"),
            "User <user@example.com>",
            "Hello",
        )
        .text("Hello")
        .html("<strong>Hello</strong>")
        .reply_to("reply@example.com")
        .header("X-Test", "yes")
        .metadata("plan", MetadataValue::String("pro".to_owned()))
        .tag("kind", "welcome")
        .build();
        message.tags.push(EmailTag {
            name: "locale".to_owned(),
            value: "en".to_owned(),
        });
        message.attachments.push(EmailAttachment {
            filename: "hello.txt".to_owned(),
            content: Some(b"hello".to_vec()),
            path: None,
            content_type: None,
            content_id: None,
            disposition: None,
        });

        let payload = to_sparkpost_payload(&message, Some(true)).await.unwrap();
        let value = serde_json::to_value(payload).unwrap();

        assert_eq!(
            value,
            json!({
                "options": { "sandbox": true },
                "recipients": [{ "address": "User <user@example.com>" }],
                "content": {
                    "from": { "email": "sender@example.com", "name": "Sender" },
                    "reply_to": "reply@example.com",
                    "subject": "Hello",
                    "html": "<strong>Hello</strong>",
                    "text": "Hello",
                    "headers": { "X-Test": "yes" },
                    "attachments": [{
                        "name": "hello.txt",
                        "type": "application/octet-stream",
                        "data": "aGVsbG8="
                    }]
                },
                "metadata": { "plan": "pro" },
                "substitution_data": { "kind": "welcome", "locale": "en" }
            })
        );
    }

    #[tokio::test]
    async fn rejects_cc() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .cc("copy@example.com")
            .build();

        let error = to_sparkpost_payload(&message, None).await.unwrap_err();

        assert_eq!(error.code, "validation_error");
        assert!(error.message.contains("cc"));
    }
}
