use std::collections::BTreeMap;

use email_sdk_core::{
    EmailMessage, EmailSdkError, MessageFieldSupport, assert_max_items,
    assert_supported_message_fields, attachment_to_base64, format_address, format_addresses,
    headers_to_object,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct JetEmailPayload {
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
    attachments: Vec<JetEmailAttachment>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct JetEmailAttachment {
    filename: String,
    data: String,
}

pub(crate) async fn to_jetemail_payload(
    message: &EmailMessage,
) -> Result<JetEmailPayload, EmailSdkError> {
    assert_supported_message_fields(
        "jetemail",
        message,
        MessageFieldSupport {
            cc: true,
            bcc: true,
            reply_to: true,
            headers: true,
            attachments: true,
            tags: false,
            metadata: false,
        },
    )?;

    let from = format_address(&message.from);
    if !from.contains('<') {
        return Err(EmailSdkError::validation(format!(
            "jetemail requires a from address with a display name, for example \"Acme <{from}>\"."
        )));
    }

    let to = format_addresses(&message.to);
    let cc = format_addresses(&message.cc);
    let bcc = format_addresses(&message.bcc);
    let reply_to = format_addresses(&message.reply_to);
    assert_max_items("jetemail", "recipient", to.len(), 50)?;
    assert_max_items("jetemail", "cc", cc.len(), 50)?;
    assert_max_items("jetemail", "bcc", bcc.len(), 50)?;
    assert_max_items("jetemail", "replyTo", reply_to.len(), 50)?;

    let mut attachments = Vec::with_capacity(message.attachments.len());
    for attachment in &message.attachments {
        attachments.push(JetEmailAttachment {
            filename: attachment.filename.clone(),
            data: attachment_to_base64(attachment).await?,
        });
    }

    Ok(JetEmailPayload {
        from,
        to,
        subject: message.subject.clone(),
        html: message.html.clone(),
        text: message.text.clone(),
        cc,
        bcc,
        reply_to,
        headers: headers_to_object(&message.headers),
        attachments,
    })
}

#[cfg(test)]
mod tests {
    use email_sdk_core::{EmailAddress, EmailAttachment, EmailMessage, MetadataValue};
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn builds_jetemail_payload() {
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
        .build();
        message.attachments.push(EmailAttachment {
            filename: "hello.txt".to_owned(),
            content: Some(b"hello".to_vec()),
            path: None,
            content_type: Some("text/plain".to_owned()),
            content_id: None,
            disposition: None,
        });

        let payload = to_jetemail_payload(&message).await.unwrap();
        let value = serde_json::to_value(payload).unwrap();

        assert_eq!(
            value,
            json!({
                "from": "Sender <sender@example.com>",
                "to": ["User <user@example.com>"],
                "subject": "Hello",
                "html": "<strong>Hello</strong>",
                "text": "Hello",
                "cc": ["copy@example.com"],
                "reply_to": ["reply@example.com"],
                "headers": { "X-Test": "yes" },
                "attachments": [{ "filename": "hello.txt", "data": "aGVsbG8=" }]
            })
        );
    }

    #[tokio::test]
    async fn rejects_bare_from_address() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .build();

        let error = to_jetemail_payload(&message).await.unwrap_err();

        assert_eq!(error.code, "validation_error");
        assert!(error.message.contains("requires a from address"));
    }

    #[tokio::test]
    async fn rejects_metadata() {
        let message = EmailMessage::builder(
            EmailAddress::named("sender@example.com", "Sender"),
            "user@example.com",
            "Hello",
        )
        .text("Hello")
        .metadata("plan", MetadataValue::String("pro".to_owned()))
        .build();

        let error = to_jetemail_payload(&message).await.unwrap_err();

        assert_eq!(error.code, "validation_error");
        assert!(error.message.contains("metadata"));
    }
}
