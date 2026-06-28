use std::collections::BTreeMap;

use email_sdk_core::{
    ApiAddress, EmailAttachmentDisposition, EmailMessage, EmailSdkError, MessageFieldSupport,
    api_address, api_addresses, assert_max_items, assert_supported_message_fields,
    attachment_to_base64, headers_to_object, metadata_to_json_object, optional_api_addresses,
    optional_single_api_address,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct MailtrapPayload {
    from: ApiAddress,
    to: Vec<ApiAddress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cc: Option<Vec<ApiAddress>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bcc: Option<Vec<ApiAddress>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_to: Option<ApiAddress>,
    subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    html: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    headers: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    custom_variables: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<MailtrapAttachment>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct MailtrapAttachment {
    filename: String,
    content: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    disposition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_id: Option<String>,
}

pub(crate) async fn to_mailtrap_payload(
    message: &EmailMessage,
) -> Result<MailtrapPayload, EmailSdkError> {
    assert_supported_message_fields(
        "mailtrap",
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
    assert_max_items("mailtrap", "tag", message.tags.len(), 1)?;

    let mut attachments = Vec::with_capacity(message.attachments.len());
    for attachment in &message.attachments {
        attachments.push(MailtrapAttachment {
            filename: attachment.filename.clone(),
            content: attachment_to_base64(attachment).await?,
            content_type: attachment.content_type.clone(),
            disposition: attachment.disposition.map(|disposition| match disposition {
                EmailAttachmentDisposition::Attachment => "attachment".to_owned(),
                EmailAttachmentDisposition::Inline => "inline".to_owned(),
            }),
            content_id: attachment.content_id.clone(),
        });
    }

    Ok(MailtrapPayload {
        from: api_address(&message.from),
        to: api_addresses(&message.to),
        cc: optional_api_addresses(&message.cc),
        bcc: optional_api_addresses(&message.bcc),
        reply_to: optional_single_api_address("mailtrap", "replyTo", &message.reply_to)?,
        subject: message.subject.clone(),
        html: message.html.clone(),
        text: message.text.clone(),
        headers: headers_to_object(&message.headers),
        custom_variables: metadata_to_json_object(&message.metadata),
        category: message.tags.first().map(|tag| tag.value.clone()),
        attachments,
    })
}

#[cfg(test)]
mod tests {
    use email_sdk_core::{
        EmailAddress, EmailAttachment, EmailAttachmentDisposition, EmailMessage, MetadataValue,
    };
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn builds_mailtrap_payload() {
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
        .tag("category", "welcome")
        .build();
        message.attachments.push(EmailAttachment {
            filename: "hello.txt".to_owned(),
            content: Some(b"hello".to_vec()),
            path: None,
            content_type: Some("text/plain".to_owned()),
            content_id: Some("hello".to_owned()),
            disposition: Some(EmailAttachmentDisposition::Inline),
        });

        let payload = to_mailtrap_payload(&message).await.unwrap();
        let value = serde_json::to_value(payload).unwrap();

        assert_eq!(
            value,
            json!({
                "from": { "email": "sender@example.com", "name": "Sender" },
                "to": [{ "email": "user@example.com", "name": "User" }],
                "cc": [{ "email": "copy@example.com" }],
                "reply_to": { "email": "reply@example.com" },
                "subject": "Hello",
                "html": "<strong>Hello</strong>",
                "text": "Hello",
                "headers": { "X-Test": "yes" },
                "custom_variables": { "plan": "pro" },
                "category": "welcome",
                "attachments": [{
                    "filename": "hello.txt",
                    "content": "aGVsbG8=",
                    "type": "text/plain",
                    "disposition": "inline",
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

        let error = to_mailtrap_payload(&message).await.unwrap_err();

        assert_eq!(error.message, "mailtrap only supports 1 tag per message.");
    }
}
