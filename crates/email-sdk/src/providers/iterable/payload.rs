use email_sdk_core::{
    EmailMessage, EmailSdkError, MessageFieldSupport, api_addresses, assert_max_items,
    assert_supported_message_fields, format_address, metadata_to_json_object,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct IterablePayload {
    #[serde(rename = "campaignId")]
    campaign_id: f64,
    #[serde(rename = "recipientEmail")]
    recipient_email: String,
    #[serde(
        rename = "allowRepeatMarketingSends",
        skip_serializing_if = "Option::is_none"
    )]
    allow_repeat_marketing_sends: Option<bool>,
    #[serde(rename = "sendAt", skip_serializing_if = "Option::is_none")]
    send_at: Option<String>,
    #[serde(rename = "dataFields")]
    data_fields: serde_json::Map<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct IterablePayloadOptions {
    pub campaign_id: f64,
    pub allow_repeat_marketing_sends: Option<bool>,
    pub data_fields: serde_json::Map<String, serde_json::Value>,
    pub send_at: Option<String>,
}

pub(crate) fn to_iterable_payload(
    message: &EmailMessage,
    options: &IterablePayloadOptions,
) -> Result<IterablePayload, EmailSdkError> {
    assert_supported_message_fields(
        "iterable",
        message,
        MessageFieldSupport {
            cc: false,
            bcc: false,
            reply_to: false,
            headers: false,
            attachments: false,
            tags: false,
            metadata: true,
        },
    )?;

    let recipients = api_addresses(&message.to);
    assert_max_items("iterable", "recipient", recipients.len(), 1)?;
    let recipient = recipients
        .first()
        .ok_or_else(|| EmailSdkError::validation("iterable requires one recipient."))?;

    let mut data_fields = options.data_fields.clone();
    data_fields.insert(
        "subject".to_owned(),
        serde_json::Value::String(message.subject.clone()),
    );
    if let Some(html) = &message.html {
        data_fields.insert("html".to_owned(), serde_json::Value::String(html.clone()));
    }
    if let Some(text) = &message.text {
        data_fields.insert("text".to_owned(), serde_json::Value::String(text.clone()));
    }
    data_fields.insert(
        "from".to_owned(),
        serde_json::Value::String(format_address(&message.from)),
    );

    Ok(IterablePayload {
        campaign_id: options.campaign_id,
        recipient_email: recipient.email.clone(),
        allow_repeat_marketing_sends: options.allow_repeat_marketing_sends,
        send_at: options.send_at.clone(),
        data_fields,
        metadata: metadata_to_json_object(&message.metadata),
    })
}

#[cfg(test)]
mod tests {
    use email_sdk_core::{EmailMessage, MetadataValue};
    use serde_json::json;

    use super::*;

    #[test]
    fn builds_iterable_payload() {
        let mut data_fields = serde_json::Map::new();
        data_fields.insert("custom".to_owned(), json!("value"));
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .html("<strong>Hello</strong>")
            .metadata("plan", MetadataValue::String("pro".to_owned()))
            .build();

        let payload = to_iterable_payload(
            &message,
            &IterablePayloadOptions {
                campaign_id: 42.0,
                allow_repeat_marketing_sends: Some(true),
                data_fields,
                send_at: Some("2026-06-28T12:00:00Z".to_owned()),
            },
        )
        .unwrap();
        let value = serde_json::to_value(payload).unwrap();

        assert_eq!(
            value,
            json!({
                "campaignId": 42.0,
                "recipientEmail": "user@example.com",
                "allowRepeatMarketingSends": true,
                "sendAt": "2026-06-28T12:00:00Z",
                "dataFields": {
                    "custom": "value",
                    "subject": "Hello",
                    "html": "<strong>Hello</strong>",
                    "text": "Hello",
                    "from": "sender@example.com"
                },
                "metadata": { "plan": "pro" }
            })
        );
    }

    #[test]
    fn rejects_multiple_recipients() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .to("other@example.com")
            .text("Hello")
            .build();

        let error = to_iterable_payload(
            &message,
            &IterablePayloadOptions {
                campaign_id: 42.0,
                ..IterablePayloadOptions::default()
            },
        )
        .unwrap_err();

        assert_eq!(
            error.message,
            "iterable only supports 1 recipient per message."
        );
    }

    #[test]
    fn rejects_headers() {
        let message = EmailMessage::builder("sender@example.com", "user@example.com", "Hello")
            .text("Hello")
            .header("X-Test", "yes")
            .build();

        let error = to_iterable_payload(
            &message,
            &IterablePayloadOptions {
                campaign_id: 42.0,
                ..IterablePayloadOptions::default()
            },
        )
        .unwrap_err();

        assert!(error.message.contains("headers"));
    }
}
