use email_sdk_core::{
    EmailAddress, EmailAttachmentDisposition, EmailMessage, EmailParts, EmailSdkError,
    MessageFieldSupport, assert_max_items, assert_supported_message_fields, attachment_to_base64,
    email_parts, headers_to_object,
};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct CloudflarePayload {
    from: CloudflareAddress,
    to: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cc: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    bcc: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_to: Option<CloudflareAddress>,
    subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    html: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    headers: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<CloudflareAttachment>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(untagged)]
enum CloudflareAddress {
    Email(String),
    Named { address: String, name: String },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct CloudflareAttachment {
    content: String,
    filename: String,
    #[serde(rename = "type")]
    content_type: String,
    disposition: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_id: Option<String>,
}

pub(crate) async fn to_cloudflare_payload(
    message: &EmailMessage,
) -> Result<CloudflarePayload, EmailSdkError> {
    assert_cloudflare_message(message)?;

    let mut attachments = Vec::with_capacity(message.attachments.len());
    for attachment in &message.attachments {
        attachments.push(CloudflareAttachment {
            content: attachment_to_base64(attachment).await?,
            filename: attachment.filename.clone(),
            content_type: attachment
                .content_type
                .clone()
                .unwrap_or_else(|| "application/octet-stream".to_owned()),
            disposition: attachment
                .disposition
                .map(|disposition| match disposition {
                    EmailAttachmentDisposition::Attachment => "attachment".to_owned(),
                    EmailAttachmentDisposition::Inline => "inline".to_owned(),
                })
                .unwrap_or_else(|| "attachment".to_owned()),
            content_id: attachment.content_id.clone(),
        });
    }

    Ok(CloudflarePayload {
        from: cloudflare_address(&message.from),
        to: cloudflare_recipients(&message.to)?,
        cc: cloudflare_recipients(&message.cc)?,
        bcc: cloudflare_recipients(&message.bcc)?,
        reply_to: cloudflare_optional_reply_to(&message.reply_to)?,
        subject: message.subject.clone(),
        html: message.html.clone(),
        text: message.text.clone(),
        headers: headers_to_object(&message.headers),
        attachments,
    })
}

fn assert_cloudflare_message(message: &EmailMessage) -> Result<(), EmailSdkError> {
    assert_supported_message_fields(
        "cloudflare",
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

    assert_max_items(
        "cloudflare",
        "recipient",
        message.to.len() + message.cc.len() + message.bcc.len(),
        50,
    )?;
    assert_max_items("cloudflare", "replyTo", message.reply_to.len(), 1)?;
    cloudflare_recipients(&message.to)?;
    cloudflare_recipients(&message.cc)?;
    cloudflare_recipients(&message.bcc)?;
    Ok(())
}

fn cloudflare_address(address: &EmailAddress) -> CloudflareAddress {
    let EmailParts { email, name } = email_parts(address);
    match name {
        Some(name) => CloudflareAddress::Named {
            address: email,
            name,
        },
        None => CloudflareAddress::Email(email),
    }
}

fn cloudflare_recipients(addresses: &[EmailAddress]) -> Result<Vec<String>, EmailSdkError> {
    addresses
        .iter()
        .map(|address| {
            let EmailParts { email, name } = email_parts(address);
            if name.is_some() {
                return Err(EmailSdkError::validation(
                    "cloudflare recipient fields only support plain email addresses.",
                ));
            }
            Ok(email)
        })
        .collect()
}

fn cloudflare_optional_reply_to(
    addresses: &[EmailAddress],
) -> Result<Option<CloudflareAddress>, EmailSdkError> {
    if addresses.is_empty() {
        Ok(None)
    } else {
        Ok(addresses.first().map(cloudflare_address))
    }
}

#[cfg(test)]
mod tests {
    use email_sdk_core::{
        EmailAddress, EmailAttachment, EmailAttachmentDisposition, EmailMessage, MetadataValue,
    };
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn builds_cloudflare_payload() {
        let mut message = EmailMessage::builder(
            EmailAddress::named("sender@example.com", "Sender"),
            "user@example.com",
            "Hello",
        )
        .text("Hello")
        .html("<strong>Hello</strong>")
        .cc("copy@example.com")
        .reply_to(EmailAddress::named("reply@example.com", "Reply"))
        .header("X-Test", "yes")
        .build();
        message.attachments.push(EmailAttachment {
            filename: "hello.txt".to_owned(),
            content: Some(b"hello".to_vec()),
            path: None,
            content_type: Some("text/plain".to_owned()),
            content_id: Some("hello".to_owned()),
            disposition: Some(EmailAttachmentDisposition::Inline),
        });

        let payload = to_cloudflare_payload(&message).await.unwrap();
        let value = serde_json::to_value(payload).unwrap();

        assert_eq!(
            value,
            json!({
                "from": { "address": "sender@example.com", "name": "Sender" },
                "to": ["user@example.com"],
                "cc": ["copy@example.com"],
                "reply_to": { "address": "reply@example.com", "name": "Reply" },
                "subject": "Hello",
                "html": "<strong>Hello</strong>",
                "text": "Hello",
                "headers": { "X-Test": "yes" },
                "attachments": [{
                    "content": "aGVsbG8=",
                    "filename": "hello.txt",
                    "type": "text/plain",
                    "disposition": "inline",
                    "content_id": "hello"
                }]
            })
        );
    }

    #[tokio::test]
    async fn rejects_named_recipient() {
        let message = EmailMessage::builder(
            "sender@example.com",
            EmailAddress::named("user@example.com", "User"),
            "Hello",
        )
        .text("Hello")
        .build();

        let error = to_cloudflare_payload(&message).await.unwrap_err();

        assert_eq!(
            error.message,
            "cloudflare recipient fields only support plain email addresses."
        );
    }

    #[tokio::test]
    async fn rejects_more_than_50_recipients() {
        let mut builder =
            EmailMessage::builder("sender@example.com", "user0@example.com", "Hello").text("Hello");
        for index in 1..51 {
            builder = builder.to(format!("user{index}@example.com"));
        }
        let message = builder.build();

        let error = to_cloudflare_payload(&message).await.unwrap_err();

        assert_eq!(
            error.message,
            "cloudflare only supports 50 recipients per message."
        );
    }

    #[tokio::test]
    async fn rejects_metadata() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .metadata("plan", MetadataValue::String("pro".to_owned()))
            .build();

        let error = to_cloudflare_payload(&message).await.unwrap_err();

        assert_eq!(error.code, "validation_error");
        assert!(error.message.contains("metadata"));
    }
}
