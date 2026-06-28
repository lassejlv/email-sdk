use email_sdk_core::{
    EmailMessage, EmailSdkError, MessageFieldSupport, assert_max_items,
    assert_supported_message_fields, attachment_to_base64, format_address, format_addresses,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct PrimitivePayload {
    from: String,
    to: String,
    subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    body_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body_html: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<PrimitiveAttachment>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct PrimitiveAttachment {
    filename: String,
    content_base64: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_type: Option<String>,
}

pub(crate) async fn to_primitive_payload(
    message: &EmailMessage,
) -> Result<PrimitivePayload, EmailSdkError> {
    assert_supported_message_fields(
        "primitive",
        message,
        MessageFieldSupport {
            cc: false,
            bcc: false,
            reply_to: false,
            headers: false,
            attachments: true,
            tags: false,
            metadata: false,
        },
    )?;

    let to = format_addresses(&message.to);
    assert_max_items("primitive", "recipient", to.len(), 1)?;
    let recipient = to
        .first()
        .cloned()
        .ok_or_else(|| EmailSdkError::validation("primitive requires one recipient."))?;

    let mut attachments = Vec::with_capacity(message.attachments.len());
    for attachment in &message.attachments {
        attachments.push(PrimitiveAttachment {
            filename: attachment.filename.clone(),
            content_base64: attachment_to_base64(attachment).await?,
            content_type: attachment.content_type.clone(),
        });
    }

    Ok(PrimitivePayload {
        from: format_address(&message.from),
        to: recipient,
        subject: message.subject.clone(),
        body_text: message.text.clone(),
        body_html: message.html.clone(),
        attachments,
    })
}

#[cfg(test)]
mod tests {
    use email_sdk_core::{EmailAttachment, EmailMessage};
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn builds_primitive_payload() {
        let mut message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .html("<strong>Hello</strong>")
            .build();
        message.attachments.push(EmailAttachment {
            filename: "hello.txt".to_owned(),
            content: Some(b"hello".to_vec()),
            path: None,
            content_type: Some("text/plain".to_owned()),
            content_id: None,
            disposition: None,
        });

        let payload = to_primitive_payload(&message).await.unwrap();
        let value = serde_json::to_value(payload).unwrap();

        assert_eq!(
            value,
            json!({
                "from": "sender@example.com",
                "to": "user@example.com",
                "subject": "Hello",
                "body_text": "Hello",
                "body_html": "<strong>Hello</strong>",
                "attachments": [{
                    "filename": "hello.txt",
                    "content_base64": "aGVsbG8=",
                    "content_type": "text/plain"
                }]
            })
        );
    }

    #[tokio::test]
    async fn rejects_multiple_recipients() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .to("other@example.com")
            .text("Hello")
            .build();

        let error = to_primitive_payload(&message).await.unwrap_err();

        assert_eq!(
            error.message,
            "primitive only supports 1 recipient per message."
        );
    }

    #[tokio::test]
    async fn rejects_headers() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .header("X-Test", "yes")
            .build();

        let error = to_primitive_payload(&message).await.unwrap_err();

        assert!(error.message.contains("headers"));
    }
}
