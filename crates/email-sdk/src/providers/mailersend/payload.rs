use email_sdk_core::{
    ApiAddress, EmailAttachmentDisposition, EmailMessage, EmailSdkError, MessageFieldSupport,
    api_address, api_addresses, assert_supported_message_fields, attachment_to_base64,
    headers_to_array, optional_api_addresses, optional_single_api_address,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct MailerSendPayload {
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    headers: Vec<MailerSendHeader>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<MailerSendAttachment>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct MailerSendHeader {
    name: String,
    value: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct MailerSendAttachment {
    filename: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    disposition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
}

pub(crate) async fn to_mailersend_payload(
    message: &EmailMessage,
) -> Result<MailerSendPayload, EmailSdkError> {
    assert_supported_message_fields(
        "mailersend",
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
        attachments.push(MailerSendAttachment {
            filename: attachment.filename.clone(),
            content: attachment_to_base64(attachment).await?,
            disposition: attachment.disposition.map(|disposition| match disposition {
                EmailAttachmentDisposition::Attachment => "attachment".to_owned(),
                EmailAttachmentDisposition::Inline => "inline".to_owned(),
            }),
            id: attachment.content_id.clone(),
        });
    }

    Ok(MailerSendPayload {
        from: api_address(&message.from),
        to: api_addresses(&message.to),
        cc: optional_api_addresses(&message.cc),
        bcc: optional_api_addresses(&message.bcc),
        reply_to: optional_single_api_address("mailersend", "replyTo", &message.reply_to)?,
        subject: message.subject.clone(),
        html: message.html.clone(),
        text: message.text.clone(),
        headers: headers_to_array(&message.headers)
            .unwrap_or_default()
            .into_iter()
            .map(|header| MailerSendHeader {
                name: header.name,
                value: header.value,
            })
            .collect(),
        attachments,
        tags: message.tags.iter().map(|tag| tag.value.clone()).collect(),
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
    async fn builds_mailersend_payload() {
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

        let payload = to_mailersend_payload(&message).await.unwrap();
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
                "headers": [{ "name": "X-Test", "value": "yes" }],
                "attachments": [{
                    "filename": "hello.txt",
                    "content": "aGVsbG8=",
                    "disposition": "inline",
                    "id": "hello"
                }],
                "tags": ["welcome"]
            })
        );
    }

    #[tokio::test]
    async fn rejects_metadata() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .metadata("plan", MetadataValue::String("pro".to_owned()))
            .build();

        let error = to_mailersend_payload(&message).await.unwrap_err();

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

        let error = to_mailersend_payload(&message).await.unwrap_err();

        assert_eq!(
            error.message,
            "mailersend only supports 1 replyTo per message."
        );
    }
}
