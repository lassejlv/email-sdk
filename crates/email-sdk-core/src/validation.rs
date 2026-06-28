use crate::{EmailMessage, EmailSdkError};

pub fn assert_message(message: &EmailMessage) -> Result<(), EmailSdkError> {
    if message.from.email().trim().is_empty() {
        return Err(EmailSdkError::validation(
            "Email message requires a from address.",
        ));
    }

    if message.to.is_empty() {
        return Err(EmailSdkError::validation(
            "Email message requires at least one recipient.",
        ));
    }

    if message.subject.trim().is_empty() {
        return Err(EmailSdkError::validation(
            "Email message requires a subject.",
        ));
    }

    if message.html.is_none() && message.text.is_none() {
        return Err(EmailSdkError::validation(
            "Email message requires either html or text content.",
        ));
    }

    for attachment in &message.attachments {
        if attachment.content.is_none() && attachment.path.is_none() {
            return Err(EmailSdkError::validation(format!(
                "Attachment \"{}\" requires content or path.",
                attachment.filename
            )));
        }
    }

    Ok(())
}
