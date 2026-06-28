use std::collections::BTreeMap;

use email_sdk_core::{
    EmailAttachment, EmailMessage, EmailSdkError, MessageFieldSupport,
    assert_supported_message_fields, attachment_to_base64, format_address, format_addresses,
    headers_to_object,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ResendPayload {
    from: String,
    to: Vec<String>,
    subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    html: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cc: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    bcc: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    reply_to: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    headers: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<ResendAttachment>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tags: Vec<ResendTag>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct ResendAttachment {
    filename: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct ResendTag {
    name: String,
    value: String,
}

pub(crate) async fn to_resend_payload(
    message: &EmailMessage,
) -> Result<ResendPayload, EmailSdkError> {
    assert_supported_message_fields(
        "resend",
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

    let mut attachments = Vec::with_capacity(message.attachments.len());
    for attachment in &message.attachments {
        attachments.push(to_resend_attachment(attachment).await?);
    }

    Ok(ResendPayload {
        from: format_address(&message.from),
        to: format_addresses(&message.to),
        subject: message.subject.clone(),
        html: message.html.clone(),
        text: message.text.clone(),
        cc: format_addresses(&message.cc),
        bcc: format_addresses(&message.bcc),
        reply_to: format_addresses(&message.reply_to),
        headers: headers_to_object(&message.headers),
        attachments,
        tags: message
            .tags
            .iter()
            .map(|tag| ResendTag {
                name: tag.name.clone(),
                value: tag.value.clone(),
            })
            .collect(),
    })
}

async fn to_resend_attachment(
    attachment: &EmailAttachment,
) -> Result<ResendAttachment, EmailSdkError> {
    Ok(ResendAttachment {
        filename: attachment.filename.clone(),
        content: attachment_to_base64(attachment).await?,
        content_type: attachment.content_type.clone(),
        content_id: attachment.content_id.clone(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use email_sdk_core::{EmailAttachment, EmailMessage, EmailTag, Headers};
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn builds_resend_payload() {
        let mut headers = BTreeMap::new();
        headers.insert("X-Test".to_owned(), "yes".to_owned());
        let mut message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .html("<strong>Hello</strong>")
            .cc("copy@example.com")
            .header("X-Ignored", "no")
            .build();
        message.headers = Some(Headers::Map(headers));
        message.attachments.push(EmailAttachment {
            filename: "hello.txt".to_owned(),
            content: Some(b"hello".to_vec()),
            path: None,
            content_type: Some("text/plain".to_owned()),
            content_id: Some("hello".to_owned()),
            disposition: None,
        });
        message.tags.push(EmailTag {
            name: "kind".to_owned(),
            value: "test".to_owned(),
        });

        let payload = to_resend_payload(&message).await.unwrap();
        let value = serde_json::to_value(payload).unwrap();

        assert_eq!(
            value,
            json!({
                "from": "sender@example.com",
                "to": ["user@example.com"],
                "subject": "Hello",
                "html": "<strong>Hello</strong>",
                "cc": ["copy@example.com"],
                "headers": { "X-Test": "yes" },
                "attachments": [{
                    "filename": "hello.txt",
                    "content": "aGVsbG8=",
                    "content_type": "text/plain",
                    "content_id": "hello"
                }],
                "tags": [{ "name": "kind", "value": "test" }]
            })
        );
    }
}
