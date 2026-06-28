use email_sdk_core::{
    ApiAddress, EmailAttachmentDisposition, EmailMessage, EmailSdkError, MessageFieldSupport,
    api_address, api_addresses, assert_supported_message_fields, attachment_to_base64,
    headers_to_object, metadata_to_json_object, optional_single_api_address,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct PlunkPayload {
    to: Vec<ApiAddress>,
    from: ApiAddress,
    subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    headers: Option<std::collections::BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<PlunkAttachment>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct PlunkAttachment {
    filename: String,
    content: String,
    #[serde(rename = "contentType")]
    content_type: String,
    #[serde(rename = "contentId", skip_serializing_if = "Option::is_none")]
    content_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    disposition: Option<String>,
}

pub(crate) async fn to_plunk_payload(
    message: &EmailMessage,
) -> Result<PlunkPayload, EmailSdkError> {
    assert_supported_message_fields(
        "plunk",
        message,
        MessageFieldSupport {
            cc: false,
            bcc: false,
            reply_to: true,
            headers: true,
            attachments: true,
            tags: false,
            metadata: true,
        },
    )?;

    let reply_to = optional_single_api_address("plunk", "replyTo", &message.reply_to)?;
    let mut attachments = Vec::with_capacity(message.attachments.len());
    for attachment in &message.attachments {
        attachments.push(PlunkAttachment {
            filename: attachment.filename.clone(),
            content: attachment_to_base64(attachment).await?,
            content_type: attachment
                .content_type
                .clone()
                .unwrap_or_else(|| "application/octet-stream".to_owned()),
            content_id: attachment.content_id.clone(),
            disposition: attachment.disposition.map(|disposition| match disposition {
                EmailAttachmentDisposition::Attachment => "attachment".to_owned(),
                EmailAttachmentDisposition::Inline => "inline".to_owned(),
            }),
        });
    }

    Ok(PlunkPayload {
        to: api_addresses(&message.to),
        from: api_address(&message.from),
        subject: message.subject.clone(),
        body: message.html.clone().or_else(|| message.text.clone()),
        data: metadata_to_json_object(&message.metadata),
        headers: headers_to_object(&message.headers),
        reply: reply_to.map(|address| address.email),
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
    async fn builds_plunk_payload() {
        let mut message = EmailMessage::builder(
            EmailAddress::named("sender@example.com", "Sender"),
            EmailAddress::named("user@example.com", "User"),
            "Hello",
        )
        .text("Hello")
        .html("<strong>Hello</strong>")
        .reply_to("reply@example.com")
        .header("X-Test", "yes")
        .metadata("plan", MetadataValue::String("pro".to_owned()))
        .build();
        message.attachments.push(EmailAttachment {
            filename: "hello.txt".to_owned(),
            content: Some(b"hello".to_vec()),
            path: None,
            content_type: Some("text/plain".to_owned()),
            content_id: Some("hello".to_owned()),
            disposition: Some(EmailAttachmentDisposition::Inline),
        });

        let payload = to_plunk_payload(&message).await.unwrap();
        let value = serde_json::to_value(payload).unwrap();

        assert_eq!(
            value,
            json!({
                "to": [{ "email": "user@example.com", "name": "User" }],
                "from": { "email": "sender@example.com", "name": "Sender" },
                "subject": "Hello",
                "body": "<strong>Hello</strong>",
                "data": { "plan": "pro" },
                "headers": { "X-Test": "yes" },
                "reply": "reply@example.com",
                "attachments": [{
                    "filename": "hello.txt",
                    "content": "aGVsbG8=",
                    "contentType": "text/plain",
                    "contentId": "hello",
                    "disposition": "inline"
                }]
            })
        );
    }

    #[tokio::test]
    async fn rejects_multiple_reply_to_addresses() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .reply_to("reply@example.com")
            .reply_to("other@example.com")
            .build();

        let error = to_plunk_payload(&message).await.unwrap_err();

        assert_eq!(error.message, "plunk only supports 1 replyTo per message.");
    }

    #[tokio::test]
    async fn rejects_cc() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .cc("copy@example.com")
            .build();

        let error = to_plunk_payload(&message).await.unwrap_err();

        assert!(error.message.contains("cc"));
    }
}
