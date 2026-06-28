use email_sdk_core::{
    EmailMessage, EmailSdkError, MessageFieldSupport, assert_supported_message_fields,
    attachment_to_base64, format_address, format_addresses, metadata_to_json_object,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct LoopsPayload {
    #[serde(rename = "transactionalId")]
    transactional_id: String,
    email: String,
    #[serde(rename = "addToAudience")]
    add_to_audience: bool,
    #[serde(rename = "dataVariables")]
    data_variables: serde_json::Map<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<LoopsAttachment>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct LoopsAttachment {
    filename: String,
    #[serde(rename = "contentType")]
    content_type: String,
    data: String,
}

pub(crate) async fn to_loops_payload(
    message: &EmailMessage,
    transactional_id: &str,
) -> Result<LoopsPayload, EmailSdkError> {
    assert_supported_message_fields(
        "loops",
        message,
        MessageFieldSupport {
            cc: false,
            bcc: false,
            reply_to: false,
            headers: false,
            attachments: true,
            tags: false,
            metadata: true,
        },
    )?;

    let recipients = format_addresses(&message.to);
    if recipients.len() != 1 {
        return Err(EmailSdkError::validation(
            "loops only supports one recipient per transactional send.",
        ));
    }

    let mut data_variables = metadata_to_json_object(&message.metadata).unwrap_or_default();
    data_variables.insert(
        "subject".to_owned(),
        serde_json::Value::String(message.subject.clone()),
    );
    if let Some(html) = &message.html {
        data_variables.insert("html".to_owned(), serde_json::Value::String(html.clone()));
    }
    if let Some(text) = &message.text {
        data_variables.insert("text".to_owned(), serde_json::Value::String(text.clone()));
    }
    data_variables.insert(
        "from".to_owned(),
        serde_json::Value::String(format_address(&message.from)),
    );

    let mut attachments = Vec::with_capacity(message.attachments.len());
    for attachment in &message.attachments {
        attachments.push(LoopsAttachment {
            filename: attachment.filename.clone(),
            content_type: attachment
                .content_type
                .clone()
                .unwrap_or_else(|| "application/octet-stream".to_owned()),
            data: attachment_to_base64(attachment).await?,
        });
    }

    Ok(LoopsPayload {
        transactional_id: transactional_id.to_owned(),
        email: recipients.into_iter().next().unwrap_or_default(),
        add_to_audience: false,
        data_variables,
        attachments,
    })
}

#[cfg(test)]
mod tests {
    use email_sdk_core::{EmailAttachment, EmailMessage, MetadataValue};
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn builds_loops_payload() {
        let mut message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .html("<strong>Hello</strong>")
            .metadata("plan", MetadataValue::String("pro".to_owned()))
            .build();
        message.attachments.push(EmailAttachment {
            filename: "hello.txt".to_owned(),
            content: Some(b"hello".to_vec()),
            path: None,
            content_type: Some("text/plain".to_owned()),
            content_id: None,
            disposition: None,
        });

        let payload = to_loops_payload(&message, "txn_123").await.unwrap();
        let value = serde_json::to_value(payload).unwrap();

        assert_eq!(
            value,
            json!({
                "transactionalId": "txn_123",
                "email": "user@example.com",
                "addToAudience": false,
                "dataVariables": {
                    "plan": "pro",
                    "subject": "Hello",
                    "html": "<strong>Hello</strong>",
                    "text": "Hello",
                    "from": "sender@example.com"
                },
                "attachments": [{
                    "filename": "hello.txt",
                    "contentType": "text/plain",
                    "data": "aGVsbG8="
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

        let error = to_loops_payload(&message, "txn_123").await.unwrap_err();

        assert_eq!(
            error.message,
            "loops only supports one recipient per transactional send."
        );
    }

    #[tokio::test]
    async fn rejects_headers() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .header("X-Test", "yes")
            .build();

        let error = to_loops_payload(&message, "txn_123").await.unwrap_err();

        assert!(error.message.contains("headers"));
    }
}
