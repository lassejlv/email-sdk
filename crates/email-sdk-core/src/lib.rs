mod client;
mod errors;
mod hooks;
mod message;
mod middleware;
mod plugin;
mod provider;
mod retry;
mod utils;
mod validation;

pub use client::{
    AdapterClient, EmailClient, EmailClientOptions, SendBatchItem, SendBatchResult, SendOptions,
    create_email_client,
};
pub use errors::{
    EmailProviderError, EmailProviderNotFoundError, EmailSdkError, EmailValidationError,
    is_retryable_email_error,
};
pub use hooks::{EmailHooks, HookEvent};
pub use message::{
    EmailAddress, EmailAttachment, EmailAttachmentDisposition, EmailHeader, EmailMessage,
    EmailMessageBuilder, EmailTag, Headers, MetadataValue,
};
pub use middleware::{
    BeforeSendEvent, BeforeSendResult, EmailSendMiddleware, SendFailureEvent, SendSuccessEvent,
};
pub use plugin::{EmailPlugin, EmailPluginContext};
pub use provider::{
    EmailProvider, EmailProviderContext, EmailProviderResponse, SharedEmailProvider,
};
pub use retry::EmailRetryConfig;
pub use utils::{
    ApiAddress, EmailParts, MessageFieldSupport, api_address, api_addresses, assert_max_items,
    assert_supported_message_fields, attachment_to_base64, attachment_to_bytes, email_parts,
    format_address, format_addresses, headers_to_array, headers_to_object, http_error_message,
    is_retryable_status, metadata_to_json_object, optional_api_addresses,
    optional_single_api_address,
};
