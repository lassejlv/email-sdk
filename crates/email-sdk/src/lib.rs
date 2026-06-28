//! Public Rust facade for email-sdk.
//!
//! ```rust,no_run
//! use email_sdk::{EmailClientOptions, EmailMessage, create_email_client};
//! use email_sdk::providers::resend::{ResendProviderOptions, resend};
//!
//! # async fn example() -> Result<(), email_sdk::EmailSdkError> {
//! let client = create_email_client(
//!     EmailClientOptions::new()
//!         .adapter(resend(ResendProviderOptions::new("re_api_key"))),
//! )?;
//!
//! let message = EmailMessage::builder("hello@example.com", "user@example.com", "Welcome")
//!     .text("Thanks for signing up.")
//!     .build();
//!
//! client.send(message, None).await?;
//! # Ok(())
//! # }
//! ```

pub use email_sdk_core::*;

pub mod providers {
    pub mod brevo;
    pub mod cloudflare;
    pub mod iterable;
    pub mod jetemail;
    pub mod lettermint;
    pub mod loops;
    pub mod mailchimp;
    pub mod mailersend;
    pub mod mailgun;
    pub mod mailpace;
    pub mod mailtrap;
    pub mod memory;
    pub mod plunk;
    pub mod postmark;
    pub mod primitive;
    pub mod resend;
    pub mod scaleway;
    pub mod sendgrid;
    pub mod sequenzy;
    pub mod ses;
    pub mod smtp;
    pub mod sparkpost;
    pub mod unosend;
    pub mod zeptomail;
}

pub use providers::{
    brevo, cloudflare, iterable, jetemail, lettermint, loops, mailchimp, mailersend, mailgun,
    mailpace, mailtrap, memory, plunk, postmark, primitive, resend, scaleway, sendgrid, sequenzy,
    ses, smtp, sparkpost, unosend, zeptomail,
};
