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
    pub mod brevo {
        pub use email_sdk_provider_brevo::*;
    }
    pub mod cloudflare {
        pub use email_sdk_provider_cloudflare::*;
    }
    pub mod iterable {
        pub use email_sdk_provider_iterable::*;
    }
    pub mod jetemail {
        pub use email_sdk_provider_jetemail::*;
    }
    pub mod lettermint {
        pub use email_sdk_provider_lettermint::*;
    }
    pub mod loops {
        pub use email_sdk_provider_loops::*;
    }
    pub mod mailchimp {
        pub use email_sdk_provider_mailchimp::*;
    }
    pub mod mailersend {
        pub use email_sdk_provider_mailersend::*;
    }
    pub mod mailgun {
        pub use email_sdk_provider_mailgun::*;
    }
    pub mod mailpace {
        pub use email_sdk_provider_mailpace::*;
    }
    pub mod mailtrap {
        pub use email_sdk_provider_mailtrap::*;
    }
    pub mod memory {
        pub use email_sdk_provider_memory::*;
    }
    pub mod plunk {
        pub use email_sdk_provider_plunk::*;
    }
    pub mod postmark {
        pub use email_sdk_provider_postmark::*;
    }
    pub mod primitive {
        pub use email_sdk_provider_primitive::*;
    }
    pub mod resend {
        pub use email_sdk_provider_resend::*;
    }
    pub mod scaleway {
        pub use email_sdk_provider_scaleway::*;
    }
    pub mod sendgrid {
        pub use email_sdk_provider_sendgrid::*;
    }
    pub mod sequenzy {
        pub use email_sdk_provider_sequenzy::*;
    }
    pub mod ses {
        pub use email_sdk_provider_ses::*;
    }
    pub mod smtp {
        pub use email_sdk_provider_smtp::*;
    }
    pub mod sparkpost {
        pub use email_sdk_provider_sparkpost::*;
    }
    pub mod unosend {
        pub use email_sdk_provider_unosend::*;
    }
    pub mod zeptomail {
        pub use email_sdk_provider_zeptomail::*;
    }
}

pub use providers::{
    brevo, cloudflare, iterable, jetemail, lettermint, loops, mailchimp, mailersend, mailgun,
    mailpace, mailtrap, memory, plunk, postmark, primitive, resend, scaleway, sendgrid, sequenzy,
    ses, smtp, sparkpost, unosend, zeptomail,
};
