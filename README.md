# email-sdk

A Rust port of the Email SDK API shape: one normalized message type, pluggable providers, middleware, hooks, retries, fallback routing, and provider-specific field validation.

This project is heavily taken from the ideas, API design, and provider behavior of [email-sdk.dev](https://email-sdk.dev). All love to Leo, who made that project. This Rust version is 100% inspired by his work.

## Install

Add the facade crate. It ships the core client and all providers.

```toml
[dependencies]
email-sdk = "0.1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

## Quick Start

```rust
use email_sdk::{EmailClientOptions, EmailMessage, create_email_client};
use email_sdk::providers::resend::{ResendProviderOptions, resend};

#[tokio::main]
async fn main() -> Result<(), email_sdk::EmailSdkError> {
    let client = create_email_client(
        EmailClientOptions::new()
            .adapter(resend(ResendProviderOptions::new("re_api_key"))),
    )?;

    let message = EmailMessage::builder("Acme <hello@example.com>", "user@example.com", "Welcome")
        .html("<strong>Hello from Rust</strong>")
        .text("Hello from Rust")
        .idempotency_key("welcome-user-123")
        .build();

    let response = client.send(message, None).await?;
    println!("{:?}", response.message_id);

    Ok(())
}
```

## Core Concepts

- `EmailMessage` is the normalized message type shared by every provider.
- Providers fail fast when a provider cannot preserve a field such as `cc`, `bcc`, `reply_to`, `headers`, `attachments`, `tags`, or `metadata`.
- The client supports named adapters, per-send adapter selection, fallback providers, retries, hooks, middleware, and plugins.
- Provider errors preserve provider name, HTTP status where relevant, retryability, and raw details.

## Providers

All providers are available from `email_sdk::providers::*`:

| Provider | Module |
| --- | --- |
| Brevo | `email_sdk::providers::brevo` |
| Cloudflare | `email_sdk::providers::cloudflare` |
| Iterable | `email_sdk::providers::iterable` |
| JetEmail | `email_sdk::providers::jetemail` |
| Lettermint | `email_sdk::providers::lettermint` |
| Loops | `email_sdk::providers::loops` |
| Mailchimp Transactional | `email_sdk::providers::mailchimp` |
| MailerSend | `email_sdk::providers::mailersend` |
| Mailgun | `email_sdk::providers::mailgun` |
| MailPace | `email_sdk::providers::mailpace` |
| Mailtrap | `email_sdk::providers::mailtrap` |
| Memory | `email_sdk::providers::memory` |
| Plunk | `email_sdk::providers::plunk` |
| Postmark | `email_sdk::providers::postmark` |
| Primitive | `email_sdk::providers::primitive` |
| Resend | `email_sdk::providers::resend` |
| Scaleway | `email_sdk::providers::scaleway` |
| SendGrid | `email_sdk::providers::sendgrid` |
| Sequenzy | `email_sdk::providers::sequenzy` |
| SES | `email_sdk::providers::ses` |
| SMTP | `email_sdk::providers::smtp` |
| SparkPost | `email_sdk::providers::sparkpost` |
| Unosend | `email_sdk::providers::unosend` |
| ZeptoMail | `email_sdk::providers::zeptomail` |

## Fallbacks and Retries

```rust
use email_sdk::{EmailClientOptions, EmailMessage, SendOptions, create_email_client};
use email_sdk::providers::memory::MemoryProvider;

let primary = MemoryProvider::named("primary");
let backup = MemoryProvider::named("backup");

let client = create_email_client(
    EmailClientOptions::new()
        .adapter(primary)
        .adapter(backup)
        .fallback_adapter("backup"),
)?;

let message = EmailMessage::builder("hello@example.com", "user@example.com", "Hello")
    .text("Hello")
    .build();

let response = client
    .send(message, Some(SendOptions::new().retries(1)))
    .await?;
# Ok::<(), email_sdk::EmailSdkError>(())
```

## Batch Sends

Use `send_many` when every message should use the same options:

```rust
use email_sdk::{EmailClientOptions, EmailMessage, create_email_client};
use email_sdk::providers::resend::{ResendProviderOptions, resend};

let client = create_email_client(
    EmailClientOptions::new()
        .adapter(resend(ResendProviderOptions::new("re_api_key"))),
)?;

let results = client
    .send_many(
        [
            EmailMessage::builder("hello@example.com", "a@example.com", "Welcome")
                .text("Hello A")
                .build(),
            EmailMessage::builder("hello@example.com", "b@example.com", "Welcome")
                .text("Hello B")
                .build(),
        ],
        None,
    )
    .await;

for result in results {
    if let Some(error) = result.error() {
        eprintln!("message {} failed: {}", result.index(), error);
    }
}
# Ok::<(), email_sdk::EmailSdkError>(())
```

Use `send_batch` with `SendBatchItem` when individual messages need adapter or fallback overrides:

```rust
use email_sdk::{
    EmailClientOptions, EmailMessage, SendBatchItem, SendOptions, create_email_client,
};

# async fn example(client: email_sdk::EmailClient) {
let results = client
    .send_batch(
        [
            SendBatchItem::new(
                EmailMessage::builder("hello@example.com", "a@example.com", "Welcome")
                    .text("Hello A")
                    .build(),
            )
            .provider("resend"),
            SendBatchItem::new(
                EmailMessage::builder("hello@example.com", "b@example.com", "Welcome")
                    .text("Hello B")
                    .build(),
            )
            .provider("smtp")
            .fallback_adapters(["resend"]),
        ],
        Some(SendOptions::new().retries(1)),
    )
    .await;
# let _ = results;
# }
```

## SMTP

The SMTP provider includes a Rust socket transport with plaintext SMTP, implicit TLS, STARTTLS, `AUTH PLAIN`, `AUTH LOGIN`, MIME body generation, envelope validation, and header-name validation.

```rust
use email_sdk::{EmailClientOptions, EmailMessage, create_email_client};
use email_sdk::providers::smtp::{SmtpAuth, SmtpProviderOptions, smtp};

let client = create_email_client(
    EmailClientOptions::new().adapter(smtp(
        SmtpProviderOptions::new("smtp.example.com")
            .port(587)
            .auth(SmtpAuth::new("user", "pass")),
    )),
)?;

let message = EmailMessage::builder("hello@example.com", "user@example.com", "Hello")
    .text("Hello")
    .build();

client.send(message, None).await?;
# Ok::<(), email_sdk::EmailSdkError>(())
```

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## License

MIT
