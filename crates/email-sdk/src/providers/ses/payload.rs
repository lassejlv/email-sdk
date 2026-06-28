use email_sdk_core::{
    EmailAttachmentDisposition, EmailMessage, EmailSdkError, MessageFieldSupport,
    assert_supported_message_fields, attachment_to_base64, format_address, format_addresses,
    headers_to_array,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct SesPayload {
    #[serde(
        rename = "ConfigurationSetName",
        skip_serializing_if = "Option::is_none"
    )]
    configuration_set_name: Option<String>,
    #[serde(rename = "FromEmailAddress")]
    from_email_address: String,
    #[serde(rename = "Destination")]
    destination: Destination,
    #[serde(rename = "ReplyToAddresses", skip_serializing_if = "Option::is_none")]
    reply_to_addresses: Option<Vec<String>>,
    #[serde(rename = "EmailTags", skip_serializing_if = "Vec::is_empty")]
    email_tags: Vec<SesTag>,
    #[serde(rename = "Content")]
    content: Content,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct Destination {
    #[serde(rename = "ToAddresses")]
    to_addresses: Vec<String>,
    #[serde(rename = "CcAddresses", skip_serializing_if = "Option::is_none")]
    cc_addresses: Option<Vec<String>>,
    #[serde(rename = "BccAddresses", skip_serializing_if = "Option::is_none")]
    bcc_addresses: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct SesTag {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Value")]
    value: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct Content {
    #[serde(rename = "Simple")]
    simple: SimpleContent,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct SimpleContent {
    #[serde(rename = "Subject")]
    subject: CharsetText,
    #[serde(rename = "Body")]
    body: Body,
    #[serde(rename = "Headers", skip_serializing_if = "Vec::is_empty")]
    headers: Vec<SesHeader>,
    #[serde(rename = "Attachments", skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<SesAttachment>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct Body {
    #[serde(rename = "Text", skip_serializing_if = "Option::is_none")]
    text: Option<CharsetText>,
    #[serde(rename = "Html", skip_serializing_if = "Option::is_none")]
    html: Option<CharsetText>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct CharsetText {
    #[serde(rename = "Data")]
    data: String,
    #[serde(rename = "Charset")]
    charset: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct SesHeader {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Value")]
    value: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct SesAttachment {
    #[serde(rename = "FileName")]
    file_name: String,
    #[serde(rename = "RawContent")]
    raw_content: String,
    #[serde(rename = "ContentType", skip_serializing_if = "Option::is_none")]
    content_type: Option<String>,
    #[serde(rename = "ContentId", skip_serializing_if = "Option::is_none")]
    content_id: Option<String>,
    #[serde(rename = "ContentDisposition", skip_serializing_if = "Option::is_none")]
    content_disposition: Option<String>,
    #[serde(rename = "ContentTransferEncoding")]
    content_transfer_encoding: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SesPayloadOptions {
    pub charset: Option<String>,
    pub configuration_set_name: Option<String>,
}

pub(crate) async fn to_ses_payload(
    message: &EmailMessage,
    options: &SesPayloadOptions,
) -> Result<SesPayload, EmailSdkError> {
    assert_supported_message_fields(
        "ses",
        message,
        MessageFieldSupport {
            cc: true,
            bcc: true,
            reply_to: true,
            headers: true,
            attachments: true,
            tags: true,
            metadata: false,
        },
    )?;

    let charset = options.charset.as_deref().unwrap_or("UTF-8");
    let mut attachments = Vec::with_capacity(message.attachments.len());
    for attachment in &message.attachments {
        attachments.push(SesAttachment {
            file_name: attachment.filename.clone(),
            raw_content: attachment_to_base64(attachment).await?,
            content_type: attachment.content_type.clone(),
            content_id: attachment.content_id.clone(),
            content_disposition: attachment.disposition.map(|disposition| match disposition {
                EmailAttachmentDisposition::Attachment => "ATTACHMENT".to_owned(),
                EmailAttachmentDisposition::Inline => "INLINE".to_owned(),
            }),
            content_transfer_encoding: "BASE64".to_owned(),
        });
    }

    Ok(SesPayload {
        configuration_set_name: options.configuration_set_name.clone(),
        from_email_address: format_address(&message.from),
        destination: Destination {
            to_addresses: format_addresses(&message.to),
            cc_addresses: optional_string_addresses(&message.cc),
            bcc_addresses: optional_string_addresses(&message.bcc),
        },
        reply_to_addresses: optional_string_addresses(&message.reply_to),
        email_tags: message
            .tags
            .iter()
            .map(|tag| SesTag {
                name: tag.name.clone(),
                value: tag.value.clone(),
            })
            .collect(),
        content: Content {
            simple: SimpleContent {
                subject: charset_text(message.subject.clone(), charset),
                body: Body {
                    text: message.text.clone().map(|text| charset_text(text, charset)),
                    html: message.html.clone().map(|html| charset_text(html, charset)),
                },
                headers: headers_to_array(&message.headers)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|header| SesHeader {
                        name: header.name,
                        value: header.value,
                    })
                    .collect(),
                attachments,
            },
        },
    })
}

fn optional_string_addresses(addresses: &[email_sdk_core::EmailAddress]) -> Option<Vec<String>> {
    if addresses.is_empty() {
        None
    } else {
        Some(format_addresses(addresses))
    }
}

fn charset_text(data: String, charset: &str) -> CharsetText {
    CharsetText {
        data,
        charset: charset.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use email_sdk_core::{
        EmailAttachment, EmailAttachmentDisposition, EmailMessage, EmailTag, MetadataValue,
    };
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn builds_ses_payload() {
        let mut message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .html("<strong>Hello</strong>")
            .cc("copy@example.com")
            .reply_to("reply@example.com")
            .header("X-Test", "yes")
            .build();
        message.tags.push(EmailTag {
            name: "kind".to_owned(),
            value: "welcome".to_owned(),
        });
        message.attachments.push(EmailAttachment {
            filename: "hello.txt".to_owned(),
            content: Some(b"hello".to_vec()),
            path: None,
            content_type: Some("text/plain".to_owned()),
            content_id: Some("hello".to_owned()),
            disposition: Some(EmailAttachmentDisposition::Attachment),
        });

        let payload = to_ses_payload(
            &message,
            &SesPayloadOptions {
                charset: Some("UTF-8".to_owned()),
                configuration_set_name: Some("config".to_owned()),
            },
        )
        .await
        .unwrap();
        let value = serde_json::to_value(payload).unwrap();

        assert_eq!(
            value,
            json!({
                "ConfigurationSetName": "config",
                "FromEmailAddress": "sender@example.com",
                "Destination": {
                    "ToAddresses": ["user@example.com"],
                    "CcAddresses": ["copy@example.com"]
                },
                "ReplyToAddresses": ["reply@example.com"],
                "EmailTags": [{ "Name": "kind", "Value": "welcome" }],
                "Content": {
                    "Simple": {
                        "Subject": { "Data": "Hello", "Charset": "UTF-8" },
                        "Body": {
                            "Text": { "Data": "Hello", "Charset": "UTF-8" },
                            "Html": { "Data": "<strong>Hello</strong>", "Charset": "UTF-8" }
                        },
                        "Headers": [{ "Name": "X-Test", "Value": "yes" }],
                        "Attachments": [{
                            "FileName": "hello.txt",
                            "RawContent": "aGVsbG8=",
                            "ContentType": "text/plain",
                            "ContentId": "hello",
                            "ContentDisposition": "ATTACHMENT",
                            "ContentTransferEncoding": "BASE64"
                        }]
                    }
                }
            })
        );
    }

    #[tokio::test]
    async fn rejects_metadata() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .metadata("plan", MetadataValue::String("pro".to_owned()))
            .build();

        let error = to_ses_payload(&message, &SesPayloadOptions::default())
            .await
            .unwrap_err();

        assert_eq!(error.code, "validation_error");
        assert!(error.message.contains("metadata"));
    }
}
