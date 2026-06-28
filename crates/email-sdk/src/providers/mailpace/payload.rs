use email_sdk_core::{
    EmailMessage, EmailSdkError, MessageFieldSupport, assert_supported_message_fields,
    format_address, format_addresses,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct MailPacePayload {
    from: String,
    to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bcc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    replyto: Option<String>,
    subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    htmlbody: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    textbody: Option<String>,
}

pub(crate) fn to_mailpace_payload(
    message: &EmailMessage,
) -> Result<MailPacePayload, EmailSdkError> {
    assert_supported_message_fields(
        "mailpace",
        message,
        MessageFieldSupport {
            cc: true,
            bcc: true,
            reply_to: true,
            headers: false,
            attachments: false,
            tags: false,
            metadata: false,
        },
    )?;

    Ok(MailPacePayload {
        from: format_address(&message.from),
        to: format_addresses(&message.to).join(","),
        cc: optional_string_addresses(&message.cc),
        bcc: optional_string_addresses(&message.bcc),
        replyto: optional_string_addresses(&message.reply_to),
        subject: message.subject.clone(),
        htmlbody: message.html.clone(),
        textbody: message.text.clone(),
    })
}

fn optional_string_addresses(addresses: &[email_sdk_core::EmailAddress]) -> Option<String> {
    if addresses.is_empty() {
        None
    } else {
        Some(format_addresses(addresses).join(","))
    }
}

#[cfg(test)]
mod tests {
    use email_sdk_core::{EmailMessage, MetadataValue};
    use serde_json::json;

    use super::*;

    #[test]
    fn builds_mailpace_payload() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .html("<strong>Hello</strong>")
            .cc("copy@example.com")
            .bcc("blind@example.com")
            .reply_to("reply@example.com")
            .build();

        let payload = to_mailpace_payload(&message).unwrap();
        let value = serde_json::to_value(payload).unwrap();

        assert_eq!(
            value,
            json!({
                "from": "sender@example.com",
                "to": "user@example.com",
                "cc": "copy@example.com",
                "bcc": "blind@example.com",
                "replyto": "reply@example.com",
                "subject": "Hello",
                "htmlbody": "<strong>Hello</strong>",
                "textbody": "Hello"
            })
        );
    }

    #[test]
    fn rejects_metadata() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .metadata("plan", MetadataValue::String("pro".to_owned()))
            .build();

        let error = to_mailpace_payload(&message).unwrap_err();

        assert!(error.message.contains("metadata"));
    }
}
