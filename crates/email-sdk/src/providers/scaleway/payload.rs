use email_sdk_core::{
    ApiAddress, EmailMessage, EmailSdkError, MessageFieldSupport, api_address, api_addresses,
    assert_supported_message_fields, attachment_to_base64, format_addresses, headers_to_array,
    optional_api_addresses,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct ScalewayPayload {
    project_id: String,
    from: ApiAddress,
    to: Vec<ApiAddress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cc: Option<Vec<ApiAddress>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bcc: Option<Vec<ApiAddress>>,
    subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    html: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    additional_headers: Vec<ScalewayHeader>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<ScalewayAttachment>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct ScalewayHeader {
    key: String,
    value: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct ScalewayAttachment {
    name: String,
    #[serde(rename = "type")]
    content_type: String,
    content: String,
}

pub(crate) async fn to_scaleway_payload(
    message: &EmailMessage,
    project_id: &str,
) -> Result<ScalewayPayload, EmailSdkError> {
    assert_supported_message_fields(
        "scaleway",
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

    let mut attachments = Vec::with_capacity(message.attachments.len());
    for attachment in &message.attachments {
        attachments.push(ScalewayAttachment {
            name: attachment.filename.clone(),
            content_type: attachment
                .content_type
                .clone()
                .unwrap_or_else(|| "application/octet-stream".to_owned()),
            content: attachment_to_base64(attachment).await?,
        });
    }

    Ok(ScalewayPayload {
        project_id: project_id.to_owned(),
        from: api_address(&message.from),
        to: api_addresses(&message.to),
        cc: optional_api_addresses(&message.cc),
        bcc: optional_api_addresses(&message.bcc),
        subject: message.subject.clone(),
        text: message.text.clone(),
        html: message.html.clone(),
        additional_headers: scaleway_headers(message)?,
        attachments,
    })
}

fn scaleway_headers(message: &EmailMessage) -> Result<Vec<ScalewayHeader>, EmailSdkError> {
    let mut headers: Vec<ScalewayHeader> = headers_to_array(&message.headers)
        .unwrap_or_default()
        .into_iter()
        .map(|header| ScalewayHeader {
            key: header.name,
            value: header.value,
        })
        .collect();

    let reply_to = format_addresses(&message.reply_to).join(", ");
    if !reply_to.is_empty() {
        if headers
            .iter()
            .any(|header| header.key.eq_ignore_ascii_case("reply-to"))
        {
            return Err(EmailSdkError::validation(
                "scaleway cannot set replyTo when headers already include Reply-To.",
            ));
        }
        headers.push(ScalewayHeader {
            key: "Reply-To".to_owned(),
            value: reply_to,
        });
    }

    Ok(headers)
}

#[cfg(test)]
mod tests {
    use email_sdk_core::{EmailAttachment, EmailMessage, MetadataValue};
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn builds_scaleway_payload() {
        let mut message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
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
            content_type: None,
            content_id: None,
            disposition: None,
        });

        let payload = to_scaleway_payload(&message, "project_123").await.unwrap();
        let value = serde_json::to_value(payload).unwrap();

        assert_eq!(
            value,
            json!({
                "project_id": "project_123",
                "from": { "email": "sender@example.com" },
                "to": [{ "email": "user@example.com" }],
                "cc": [{ "email": "copy@example.com" }],
                "subject": "Hello",
                "text": "Hello",
                "html": "<strong>Hello</strong>",
                "additional_headers": [
                    { "key": "X-Test", "value": "yes" },
                    { "key": "Reply-To", "value": "reply@example.com" }
                ],
                "attachments": [{
                    "name": "hello.txt",
                    "type": "application/octet-stream",
                    "content": "aGVsbG8="
                }]
            })
        );
    }

    #[tokio::test]
    async fn rejects_reply_to_header_conflict() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .reply_to("reply@example.com")
            .header("Reply-To", "existing@example.com")
            .build();

        let error = to_scaleway_payload(&message, "project_123")
            .await
            .unwrap_err();

        assert_eq!(
            error.message,
            "scaleway cannot set replyTo when headers already include Reply-To."
        );
    }

    #[tokio::test]
    async fn rejects_metadata() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .metadata("plan", MetadataValue::String("pro".to_owned()))
            .build();

        let error = to_scaleway_payload(&message, "project_123")
            .await
            .unwrap_err();

        assert_eq!(error.code, "validation_error");
        assert!(error.message.contains("metadata"));
    }
}
