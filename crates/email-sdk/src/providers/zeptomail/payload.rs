use email_sdk_core::{
    EmailAddress, EmailMessage, EmailParts, EmailSdkError, MessageFieldSupport,
    assert_supported_message_fields, attachment_to_base64, email_parts,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct ZeptoMailPayload {
    from: ZeptoAddress,
    to: Vec<ZeptoRecipient>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cc: Vec<ZeptoRecipient>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    bcc: Vec<ZeptoRecipient>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    reply_to: Vec<ZeptoRecipient>,
    subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    htmlbody: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    textbody: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<ZeptoAttachment>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct ZeptoRecipient {
    email_address: ZeptoAddress,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct ZeptoAddress {
    address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct ZeptoAttachment {
    name: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    mime_type: Option<String>,
}

pub(crate) async fn to_zeptomail_payload(
    message: &EmailMessage,
) -> Result<ZeptoMailPayload, EmailSdkError> {
    assert_supported_message_fields(
        "zeptomail",
        message,
        MessageFieldSupport {
            cc: true,
            bcc: true,
            reply_to: true,
            headers: false,
            attachments: true,
            tags: false,
            metadata: false,
        },
    )?;

    let mut attachments = Vec::with_capacity(message.attachments.len());
    for attachment in &message.attachments {
        attachments.push(ZeptoAttachment {
            name: attachment.filename.clone(),
            content: attachment_to_base64(attachment).await?,
            mime_type: attachment.content_type.clone(),
        });
    }

    Ok(ZeptoMailPayload {
        from: zepto_address(&message.from),
        to: zepto_recipients(&message.to),
        cc: zepto_recipients(&message.cc),
        bcc: zepto_recipients(&message.bcc),
        reply_to: zepto_recipients(&message.reply_to),
        subject: message.subject.clone(),
        htmlbody: message.html.clone(),
        textbody: message.text.clone(),
        attachments,
    })
}

fn zepto_address(address: &EmailAddress) -> ZeptoAddress {
    let EmailParts { email, name } = email_parts(address);
    ZeptoAddress {
        address: email,
        name,
    }
}

fn zepto_recipients(addresses: &[EmailAddress]) -> Vec<ZeptoRecipient> {
    addresses
        .iter()
        .map(|address| ZeptoRecipient {
            email_address: zepto_address(address),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use email_sdk_core::{EmailAddress, EmailAttachment, EmailMessage, MetadataValue};
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn builds_zeptomail_payload() {
        let mut message = EmailMessage::builder(
            EmailAddress::named("sender@example.com", "Sender"),
            EmailAddress::named("user@example.com", "User"),
            "Hello",
        )
        .text("Hello")
        .html("<strong>Hello</strong>")
        .cc("copy@example.com")
        .reply_to("reply@example.com")
        .build();
        message.attachments.push(EmailAttachment {
            filename: "hello.txt".to_owned(),
            content: Some(b"hello".to_vec()),
            path: None,
            content_type: Some("text/plain".to_owned()),
            content_id: None,
            disposition: None,
        });

        let payload = to_zeptomail_payload(&message).await.unwrap();
        let value = serde_json::to_value(payload).unwrap();

        assert_eq!(
            value,
            json!({
                "from": { "address": "sender@example.com", "name": "Sender" },
                "to": [{ "email_address": { "address": "user@example.com", "name": "User" } }],
                "cc": [{ "email_address": { "address": "copy@example.com" } }],
                "reply_to": [{ "email_address": { "address": "reply@example.com" } }],
                "subject": "Hello",
                "htmlbody": "<strong>Hello</strong>",
                "textbody": "Hello",
                "attachments": [{ "name": "hello.txt", "content": "aGVsbG8=", "mime_type": "text/plain" }]
            })
        );
    }

    #[tokio::test]
    async fn rejects_headers() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .header("X-Test", "yes")
            .build();

        let error = to_zeptomail_payload(&message).await.unwrap_err();

        assert_eq!(error.code, "validation_error");
        assert!(error.message.contains("headers"));
    }

    #[tokio::test]
    async fn rejects_metadata() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .metadata("plan", MetadataValue::String("pro".to_owned()))
            .build();

        let error = to_zeptomail_payload(&message).await.unwrap_err();

        assert_eq!(error.code, "validation_error");
        assert!(error.message.contains("metadata"));
    }
}
