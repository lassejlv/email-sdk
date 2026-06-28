use email_sdk_core::{
    EmailAttachment, EmailMessage, EmailSdkError, EmailTag, assert_max_items, attachment_to_base64,
    format_address, format_addresses, headers_to_array, metadata_to_json_object,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct PostmarkPayload {
    #[serde(rename = "From")]
    from: String,
    #[serde(rename = "To")]
    to: String,
    #[serde(rename = "Cc", skip_serializing_if = "Option::is_none")]
    cc: Option<String>,
    #[serde(rename = "Bcc", skip_serializing_if = "Option::is_none")]
    bcc: Option<String>,
    #[serde(rename = "ReplyTo", skip_serializing_if = "Option::is_none")]
    reply_to: Option<String>,
    #[serde(rename = "Subject")]
    subject: String,
    #[serde(rename = "HtmlBody", skip_serializing_if = "Option::is_none")]
    html_body: Option<String>,
    #[serde(rename = "TextBody", skip_serializing_if = "Option::is_none")]
    text_body: Option<String>,
    #[serde(rename = "Headers", skip_serializing_if = "Vec::is_empty")]
    headers: Vec<PostmarkHeader>,
    #[serde(rename = "Attachments", skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<PostmarkAttachment>,
    #[serde(rename = "Metadata", skip_serializing_if = "Option::is_none")]
    metadata: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(rename = "MessageStream", skip_serializing_if = "Option::is_none")]
    message_stream: Option<String>,
    #[serde(rename = "Tag", skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct PostmarkHeader {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Value")]
    value: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct PostmarkAttachment {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Content")]
    content: String,
    #[serde(rename = "ContentType")]
    content_type: String,
    #[serde(rename = "ContentID", skip_serializing_if = "Option::is_none")]
    content_id: Option<String>,
}

pub(crate) async fn to_postmark_payload(
    message: &EmailMessage,
    message_stream: Option<&str>,
) -> Result<PostmarkPayload, EmailSdkError> {
    let mut attachments = Vec::with_capacity(message.attachments.len());
    for attachment in &message.attachments {
        attachments.push(to_postmark_attachment(attachment).await?);
    }

    Ok(PostmarkPayload {
        from: format_address(&message.from),
        to: format_addresses(&message.to).join(", "),
        cc: joined_addresses(&message.cc),
        bcc: joined_addresses(&message.bcc),
        reply_to: joined_addresses(&message.reply_to),
        subject: message.subject.clone(),
        html_body: message.html.clone(),
        text_body: message.text.clone(),
        headers: headers_to_array(&message.headers)
            .unwrap_or_default()
            .into_iter()
            .map(|header| PostmarkHeader {
                name: header.name,
                value: header.value,
            })
            .collect(),
        attachments,
        metadata: metadata_to_json_object(&message.metadata),
        message_stream: message_stream.map(ToOwned::to_owned),
        tag: first_postmark_tag(&message.tags)?,
    })
}

async fn to_postmark_attachment(
    attachment: &EmailAttachment,
) -> Result<PostmarkAttachment, EmailSdkError> {
    Ok(PostmarkAttachment {
        name: attachment.filename.clone(),
        content: attachment_to_base64(attachment).await?,
        content_type: attachment
            .content_type
            .clone()
            .unwrap_or_else(|| "application/octet-stream".to_owned()),
        content_id: attachment.content_id.clone(),
    })
}

fn joined_addresses(addresses: &[email_sdk_core::EmailAddress]) -> Option<String> {
    let formatted = format_addresses(addresses).join(", ");
    if formatted.is_empty() {
        None
    } else {
        Some(formatted)
    }
}

fn first_postmark_tag(tags: &[EmailTag]) -> Result<Option<String>, EmailSdkError> {
    if tags.is_empty() {
        return Ok(None);
    }

    assert_max_items("postmark", "tag", tags.len(), 1)?;
    Ok(tags
        .first()
        .map(|tag| format!("{}:{}", tag.name, tag.value)))
}

#[cfg(test)]
mod tests {
    use email_sdk_core::{EmailAttachment, EmailMessage, EmailTag, MetadataValue};
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn builds_postmark_payload() {
        let mut message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .html("<strong>Hello</strong>")
            .text("Hello")
            .cc("copy@example.com")
            .reply_to("reply@example.com")
            .header("X-Test", "yes")
            .metadata("plan", MetadataValue::String("pro".to_owned()))
            .build();
        message.attachments.push(EmailAttachment {
            filename: "hello.txt".to_owned(),
            content: Some(b"hello".to_vec()),
            path: None,
            content_type: None,
            content_id: Some("hello".to_owned()),
            disposition: None,
        });
        message.tags.push(EmailTag {
            name: "kind".to_owned(),
            value: "welcome".to_owned(),
        });

        let payload = to_postmark_payload(&message, Some("outbound"))
            .await
            .unwrap();
        let value = serde_json::to_value(payload).unwrap();

        assert_eq!(
            value,
            json!({
                "From": "sender@example.com",
                "To": "user@example.com",
                "Cc": "copy@example.com",
                "ReplyTo": "reply@example.com",
                "Subject": "Hello",
                "HtmlBody": "<strong>Hello</strong>",
                "TextBody": "Hello",
                "Headers": [{ "Name": "X-Test", "Value": "yes" }],
                "Attachments": [{
                    "Name": "hello.txt",
                    "Content": "aGVsbG8=",
                    "ContentType": "application/octet-stream",
                    "ContentID": "hello"
                }],
                "Metadata": { "plan": "pro" },
                "MessageStream": "outbound",
                "Tag": "kind:welcome"
            })
        );
    }

    #[tokio::test]
    async fn rejects_multiple_tags() {
        let mut message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .build();
        message.tags.push(EmailTag {
            name: "a".to_owned(),
            value: "1".to_owned(),
        });
        message.tags.push(EmailTag {
            name: "b".to_owned(),
            value: "2".to_owned(),
        });

        let error = to_postmark_payload(&message, None).await.unwrap_err();

        assert_eq!(error.code, "validation_error");
        assert_eq!(error.message, "postmark only supports 1 tag per message.");
    }
}
