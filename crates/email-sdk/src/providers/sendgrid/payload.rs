use std::collections::BTreeMap;

use email_sdk_core::{
    ApiAddress, EmailAttachmentDisposition, EmailMessage, EmailSdkError, api_address,
    api_addresses, attachment_to_base64, headers_to_object, metadata_to_json_object,
    optional_api_addresses,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct SendGridPayload {
    personalizations: Vec<Personalization>,
    from: ApiAddress,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_to_list: Option<Vec<ApiAddress>>,
    subject: String,
    content: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attachments: Option<Vec<SendGridAttachment>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    categories: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct Personalization {
    to: Vec<ApiAddress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cc: Option<Vec<ApiAddress>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bcc: Option<Vec<ApiAddress>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    headers: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    custom_args: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct Content {
    #[serde(rename = "type")]
    content_type: String,
    value: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct SendGridAttachment {
    filename: String,
    content: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    disposition: Option<String>,
}

pub(crate) async fn to_sendgrid_payload(
    message: &EmailMessage,
) -> Result<SendGridPayload, EmailSdkError> {
    Ok(SendGridPayload {
        personalizations: vec![Personalization {
            to: api_addresses(&message.to),
            cc: optional_api_addresses(&message.cc),
            bcc: optional_api_addresses(&message.bcc),
            headers: headers_to_object(&message.headers),
            custom_args: metadata_to_json_object(&message.metadata),
        }],
        from: api_address(&message.from),
        reply_to_list: optional_api_addresses(&message.reply_to),
        subject: message.subject.clone(),
        content: content(message),
        attachments: sendgrid_attachments(message).await?,
        categories: categories(message),
    })
}

fn content(message: &EmailMessage) -> Vec<Content> {
    let mut content = Vec::new();
    if let Some(text) = &message.text {
        content.push(Content {
            content_type: "text/plain".to_owned(),
            value: text.clone(),
        });
    }
    if let Some(html) = &message.html {
        content.push(Content {
            content_type: "text/html".to_owned(),
            value: html.clone(),
        });
    }
    content
}

async fn sendgrid_attachments(
    message: &EmailMessage,
) -> Result<Option<Vec<SendGridAttachment>>, EmailSdkError> {
    if message.attachments.is_empty() {
        return Ok(None);
    }

    let mut attachments = Vec::with_capacity(message.attachments.len());
    for attachment in &message.attachments {
        attachments.push(SendGridAttachment {
            filename: attachment.filename.clone(),
            content: attachment_to_base64(attachment).await?,
            content_type: attachment.content_type.clone(),
            content_id: attachment.content_id.clone(),
            disposition: attachment.disposition.map(|disposition| match disposition {
                EmailAttachmentDisposition::Attachment => "attachment".to_owned(),
                EmailAttachmentDisposition::Inline => "inline".to_owned(),
            }),
        });
    }

    Ok(Some(attachments))
}

fn categories(message: &EmailMessage) -> Option<Vec<String>> {
    if message.tags.is_empty() {
        return None;
    }

    Some(message.tags.iter().map(|tag| tag.value.clone()).collect())
}

#[cfg(test)]
mod tests {
    use email_sdk_core::{
        EmailAddress, EmailAttachment, EmailAttachmentDisposition, EmailMessage, EmailTag,
        MetadataValue,
    };
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn builds_sendgrid_payload() {
        let mut message = EmailMessage::builder(
            EmailAddress::named("sender@example.com", "Sender"),
            EmailAddress::named("user@example.com", "User"),
            "Hello",
        )
        .text("Hello")
        .html("<strong>Hello</strong>")
        .cc("copy@example.com")
        .bcc("blind@example.com")
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
        message.tags.push(EmailTag {
            name: "kind".to_owned(),
            value: "welcome".to_owned(),
        });

        let payload = to_sendgrid_payload(&message).await.unwrap();
        let value = serde_json::to_value(payload).unwrap();

        assert_eq!(
            value,
            json!({
                "personalizations": [{
                    "to": [{ "email": "user@example.com", "name": "User" }],
                    "cc": [{ "email": "copy@example.com" }],
                    "bcc": [{ "email": "blind@example.com" }],
                    "headers": { "X-Test": "yes" },
                    "custom_args": { "plan": "pro" }
                }],
                "from": { "email": "sender@example.com", "name": "Sender" },
                "reply_to_list": [{ "email": "reply@example.com" }],
                "subject": "Hello",
                "content": [
                    { "type": "text/plain", "value": "Hello" },
                    { "type": "text/html", "value": "<strong>Hello</strong>" }
                ],
                "attachments": [{
                    "filename": "hello.txt",
                    "content": "aGVsbG8=",
                    "type": "text/plain",
                    "content_id": "hello",
                    "disposition": "inline"
                }],
                "categories": ["welcome"]
            })
        );
    }
}
