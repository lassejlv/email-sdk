use email_sdk_core::{
    ApiAddress, EmailMessage, EmailSdkError, api_address, api_addresses, attachment_to_base64,
    headers_to_object, metadata_to_json_object, optional_api_addresses,
    optional_single_api_address,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct BrevoPayload {
    sender: ApiAddress,
    to: Vec<ApiAddress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cc: Option<Vec<ApiAddress>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bcc: Option<Vec<ApiAddress>>,
    #[serde(rename = "replyTo", skip_serializing_if = "Option::is_none")]
    reply_to: Option<ApiAddress>,
    subject: String,
    #[serde(rename = "htmlContent", skip_serializing_if = "Option::is_none")]
    html_content: Option<String>,
    #[serde(rename = "textContent", skip_serializing_if = "Option::is_none")]
    text_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    headers: Option<std::collections::BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    #[serde(rename = "attachment", skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<BrevoAttachment>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct BrevoAttachment {
    name: String,
    content: String,
}

pub(crate) async fn to_brevo_payload(
    message: &EmailMessage,
) -> Result<BrevoPayload, EmailSdkError> {
    let mut attachments = Vec::with_capacity(message.attachments.len());
    for attachment in &message.attachments {
        attachments.push(BrevoAttachment {
            name: attachment.filename.clone(),
            content: attachment_to_base64(attachment).await?,
        });
    }

    Ok(BrevoPayload {
        sender: api_address(&message.from),
        to: api_addresses(&message.to),
        cc: optional_api_addresses(&message.cc),
        bcc: optional_api_addresses(&message.bcc),
        reply_to: optional_single_api_address("brevo", "replyTo", &message.reply_to)?,
        subject: message.subject.clone(),
        html_content: message.html.clone(),
        text_content: message.text.clone(),
        headers: headers_to_object(&message.headers),
        params: metadata_to_json_object(&message.metadata),
        tags: message.tags.iter().map(|tag| tag.value.clone()).collect(),
        attachments,
    })
}

#[cfg(test)]
mod tests {
    use email_sdk_core::{EmailAddress, EmailAttachment, EmailMessage, MetadataValue};
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn builds_brevo_payload() {
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
        .tag("kind", "welcome")
        .build();
        message.attachments.push(EmailAttachment {
            filename: "hello.txt".to_owned(),
            content: Some(b"hello".to_vec()),
            path: None,
            content_type: Some("text/plain".to_owned()),
            content_id: None,
            disposition: None,
        });

        let payload = to_brevo_payload(&message).await.unwrap();
        let value = serde_json::to_value(payload).unwrap();

        assert_eq!(
            value,
            json!({
                "sender": { "email": "sender@example.com", "name": "Sender" },
                "to": [{ "email": "user@example.com", "name": "User" }],
                "cc": [{ "email": "copy@example.com" }],
                "replyTo": { "email": "reply@example.com" },
                "subject": "Hello",
                "htmlContent": "<strong>Hello</strong>",
                "textContent": "Hello",
                "headers": { "X-Test": "yes" },
                "params": { "plan": "pro" },
                "tags": ["welcome"],
                "attachment": [{ "name": "hello.txt", "content": "aGVsbG8=" }]
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

        let error = to_brevo_payload(&message).await.unwrap_err();

        assert_eq!(error.message, "brevo only supports 1 replyTo per message.");
    }
}
