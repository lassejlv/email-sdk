use email_sdk_core::{
    EmailAddress, EmailMessage, EmailSdkError, MessageFieldSupport,
    assert_supported_message_fields, attachment_to_base64, email_parts, headers_to_object,
    metadata_to_json_object,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct MailchimpPayload {
    key: String,
    message: MailchimpMessage,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct MailchimpMessage {
    from_email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    from_name: Option<String>,
    to: Vec<MailchimpRecipient>,
    #[serde(skip_serializing_if = "Option::is_none")]
    headers: Option<std::collections::BTreeMap<String, String>>,
    subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    html: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<MailchimpAttachment>,
    important: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct MailchimpRecipient {
    email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(rename = "type")]
    recipient_type: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct MailchimpAttachment {
    #[serde(rename = "type")]
    content_type: String,
    name: String,
    content: String,
}

pub(crate) async fn to_mailchimp_payload(
    message: &EmailMessage,
    api_key: &str,
) -> Result<MailchimpPayload, EmailSdkError> {
    assert_supported_message_fields(
        "mailchimp",
        message,
        MessageFieldSupport {
            cc: true,
            bcc: true,
            reply_to: false,
            headers: true,
            attachments: true,
            tags: true,
            metadata: true,
        },
    )?;

    let from = email_parts(&message.from);
    let mut recipients = Vec::new();
    recipients.extend(mailchimp_recipients(&message.to, "to"));
    recipients.extend(mailchimp_recipients(&message.cc, "cc"));
    recipients.extend(mailchimp_recipients(&message.bcc, "bcc"));

    let mut attachments = Vec::with_capacity(message.attachments.len());
    for attachment in &message.attachments {
        attachments.push(MailchimpAttachment {
            content_type: attachment
                .content_type
                .clone()
                .unwrap_or_else(|| "application/octet-stream".to_owned()),
            name: attachment.filename.clone(),
            content: attachment_to_base64(attachment).await?,
        });
    }

    Ok(MailchimpPayload {
        key: api_key.to_owned(),
        message: MailchimpMessage {
            from_email: from.email,
            from_name: from.name,
            to: recipients,
            headers: headers_to_object(&message.headers),
            subject: message.subject.clone(),
            html: message.html.clone(),
            text: message.text.clone(),
            metadata: metadata_to_json_object(&message.metadata),
            tags: message.tags.iter().map(|tag| tag.value.clone()).collect(),
            attachments,
            important: false,
        },
    })
}

fn mailchimp_recipients(
    addresses: &[EmailAddress],
    recipient_type: &'static str,
) -> Vec<MailchimpRecipient> {
    addresses
        .iter()
        .map(|address| {
            let parsed = email_parts(address);
            MailchimpRecipient {
                email: parsed.email,
                name: parsed.name,
                recipient_type,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use email_sdk_core::{EmailAttachment, EmailMessage, EmailTag, MetadataValue};
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn builds_mailchimp_payload() {
        let mut message = EmailMessage::builder(
            "\"Sender\" <sender@example.com>",
            "User <user@example.com>",
            "Hello",
        )
        .text("Hello")
        .html("<strong>Hello</strong>")
        .cc("copy@example.com")
        .bcc("Blind <blind@example.com>")
        .header("X-Test", "yes")
        .metadata("plan", MetadataValue::String("pro".to_owned()))
        .tag("kind", "welcome")
        .build();
        message.tags.push(EmailTag {
            name: "ignored".to_owned(),
            value: "receipt".to_owned(),
        });
        message.attachments.push(EmailAttachment {
            filename: "hello.txt".to_owned(),
            content: Some(b"hello".to_vec()),
            path: None,
            content_type: None,
            content_id: None,
            disposition: None,
        });

        let payload = to_mailchimp_payload(&message, "key").await.unwrap();
        let value = serde_json::to_value(payload).unwrap();

        assert_eq!(
            value,
            json!({
                "key": "key",
                "message": {
                    "from_email": "sender@example.com",
                    "from_name": "Sender",
                    "to": [
                        { "email": "user@example.com", "name": "User", "type": "to" },
                        { "email": "copy@example.com", "type": "cc" },
                        { "email": "blind@example.com", "name": "Blind", "type": "bcc" }
                    ],
                    "headers": { "X-Test": "yes" },
                    "subject": "Hello",
                    "html": "<strong>Hello</strong>",
                    "text": "Hello",
                    "metadata": { "plan": "pro" },
                    "tags": ["welcome", "receipt"],
                    "attachments": [{
                        "type": "application/octet-stream",
                        "name": "hello.txt",
                        "content": "aGVsbG8="
                    }],
                    "important": false
                }
            })
        );
    }

    #[tokio::test]
    async fn rejects_reply_to() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .reply_to("reply@example.com")
            .build();

        let error = to_mailchimp_payload(&message, "key").await.unwrap_err();

        assert_eq!(error.code, "validation_error");
        assert!(error.message.contains("replyTo"));
    }
}
