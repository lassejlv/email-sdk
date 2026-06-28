use email_sdk_core::{
    EmailAttachmentDisposition, EmailMessage, EmailSdkError, Headers, MetadataValue,
    attachment_to_bytes, format_address, format_addresses,
};
use reqwest::multipart::{Form, Part};

pub(crate) async fn to_mailgun_form(message: &EmailMessage) -> Result<Form, EmailSdkError> {
    let mut form = Form::new()
        .text("from", format_address(&message.from))
        .text("subject", message.subject.clone());

    for to in format_addresses(&message.to) {
        form = form.text("to", to);
    }
    for cc in format_addresses(&message.cc) {
        form = form.text("cc", cc);
    }
    for bcc in format_addresses(&message.bcc) {
        form = form.text("bcc", bcc);
    }
    for reply_to in format_addresses(&message.reply_to) {
        form = form.text("h:Reply-To", reply_to);
    }

    form = append_headers(form, message);

    if let Some(text) = &message.text {
        form = form.text("text", text.clone());
    }
    if let Some(html) = &message.html {
        form = form.text("html", html.clone());
    }

    for (name, value) in &message.metadata {
        form = form.text(format!("v:{name}"), metadata_value_to_string(value));
    }

    for tag in &message.tags {
        form = form.text("o:tag", tag.value.clone());
    }

    for attachment in &message.attachments {
        let bytes = attachment_to_bytes(attachment).await?;
        let content_type = attachment
            .content_type
            .as_deref()
            .unwrap_or("application/octet-stream");
        let part = Part::bytes(bytes)
            .file_name(attachment.filename.clone())
            .mime_str(content_type)
            .map_err(|error| {
                EmailSdkError::validation(format!(
                    "Attachment \"{}\" has invalid content type: {error}.",
                    attachment.filename
                ))
            })?;
        let field = match attachment.disposition {
            Some(EmailAttachmentDisposition::Inline) => "inline",
            _ => "attachment",
        };
        form = form.part(field, part);
    }

    Ok(form)
}

fn append_headers(mut form: Form, message: &EmailMessage) -> Form {
    match &message.headers {
        Some(Headers::Map(headers)) => {
            for (name, value) in headers {
                form = form.text(format!("h:{name}"), value.clone());
            }
        }
        Some(Headers::List(headers)) => {
            for header in headers {
                form = form.text(format!("h:{}", header.name), header.value.clone());
            }
        }
        None => {}
    }

    form
}

fn metadata_value_to_string(value: &MetadataValue) -> String {
    match value {
        MetadataValue::String(value) => value.clone(),
        MetadataValue::Number(value) => value.to_string(),
        MetadataValue::Bool(value) => value.to_string(),
        MetadataValue::Null => "null".to_owned(),
    }
}
