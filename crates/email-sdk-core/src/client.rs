use std::{collections::HashMap, sync::Arc, time::Duration};

use tokio::time::sleep;

use crate::{
    BeforeSendEvent, EmailHooks, EmailMessage, EmailPlugin, EmailPluginContext,
    EmailProviderContext, EmailProviderResponse, EmailRetryConfig, EmailSdkError,
    EmailSendMiddleware, HookEvent, SendFailureEvent, SendSuccessEvent, SharedEmailProvider,
    provider::EmailProvider, validation::assert_message,
};

#[derive(Clone, Default)]
pub struct EmailClientOptions {
    pub adapters: Vec<SharedEmailProvider>,
    pub default_adapter: Option<String>,
    pub fallback: Vec<String>,
    pub retry: EmailRetryConfig,
    pub hooks: Vec<Arc<dyn EmailHooks>>,
    pub plugins: Vec<Arc<dyn EmailPlugin>>,
}

impl EmailClientOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn adapter<T>(mut self, adapter: Arc<T>) -> Self
    where
        T: EmailProvider + 'static,
    {
        self.adapters.push(adapter);
        self
    }

    pub fn shared_adapter(mut self, adapter: SharedEmailProvider) -> Self {
        self.adapters.push(adapter);
        self
    }

    pub fn provider<T>(self, provider: Arc<T>) -> Self
    where
        T: EmailProvider + 'static,
    {
        self.adapter(provider)
    }

    pub fn shared_provider(self, provider: SharedEmailProvider) -> Self {
        self.shared_adapter(provider)
    }

    pub fn default_adapter(mut self, name: impl Into<String>) -> Self {
        self.default_adapter = Some(name.into());
        self
    }

    pub fn default_provider(self, name: impl Into<String>) -> Self {
        self.default_adapter(name)
    }

    pub fn fallback(mut self, names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.fallback = names.into_iter().map(Into::into).collect();
        self
    }

    pub fn retry(mut self, retry: EmailRetryConfig) -> Self {
        self.retry = retry;
        self
    }

    pub fn hooks<T>(mut self, hooks: Arc<T>) -> Self
    where
        T: EmailHooks + 'static,
    {
        self.hooks.push(hooks);
        self
    }

    pub fn plugin<T>(mut self, plugin: Arc<T>) -> Self
    where
        T: EmailPlugin + 'static,
    {
        self.plugins.push(plugin);
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SendOptions {
    pub adapter: Option<String>,
    pub fallback_adapters: Option<Vec<String>>,
    pub retries: Option<usize>,
    pub idempotency_key: Option<String>,
    pub metadata: HashMap<String, String>,
}

impl SendOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn adapter(mut self, adapter: impl Into<String>) -> Self {
        self.adapter = Some(adapter.into());
        self
    }

    pub fn provider(self, provider: impl Into<String>) -> Self {
        self.adapter(provider)
    }

    pub fn fallback_adapters(
        mut self,
        adapters: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.fallback_adapters = Some(adapters.into_iter().map(Into::into).collect());
        self
    }

    pub fn retries(mut self, retries: usize) -> Self {
        self.retries = Some(retries);
        self
    }

    pub fn idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SendBatchItem {
    pub message: EmailMessage,
    pub adapter: Option<String>,
    pub fallback_adapters: Option<Vec<String>>,
}

impl SendBatchItem {
    pub fn new(message: EmailMessage) -> Self {
        Self {
            message,
            adapter: None,
            fallback_adapters: None,
        }
    }

    pub fn adapter(mut self, adapter: impl Into<String>) -> Self {
        self.adapter = Some(adapter.into());
        self
    }

    pub fn provider(self, provider: impl Into<String>) -> Self {
        self.adapter(provider)
    }

    pub fn fallback_adapters(
        mut self,
        adapters: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.fallback_adapters = Some(adapters.into_iter().map(Into::into).collect());
        self
    }
}

#[derive(Debug, Clone)]
pub enum SendBatchResult {
    Ok {
        index: usize,
        response: EmailProviderResponse,
    },
    Err {
        index: usize,
        error: EmailSdkError,
    },
}

#[derive(Clone)]
pub struct EmailClient {
    adapters: HashMap<String, SharedEmailProvider>,
    default_adapter: String,
    fallback: Vec<String>,
    retry: EmailRetryConfig,
    hooks: Vec<Arc<dyn EmailHooks>>,
    middleware: Vec<Arc<dyn EmailSendMiddleware>>,
}

pub fn create_email_client(options: EmailClientOptions) -> Result<EmailClient, EmailSdkError> {
    EmailClient::new(options)
}

impl EmailClient {
    pub fn new(options: EmailClientOptions) -> Result<Self, EmailSdkError> {
        let mut adapters = HashMap::new();
        for adapter in options.adapters {
            add_adapter(&mut adapters, adapter)?;
        }

        let requested_default = options
            .default_adapter
            .clone()
            .or_else(|| adapters.keys().next().cloned())
            .unwrap_or_default();

        let mut plugin_ids = std::collections::HashSet::new();
        let mut hooks = Vec::new();
        let mut middleware = Vec::new();

        for plugin in &options.plugins {
            if !plugin_ids.insert(plugin.id().to_owned()) {
                return Err(EmailSdkError::validation(format!(
                    "Duplicate email plugin \"{}\".",
                    plugin.id()
                )));
            }

            let mut context = EmailPluginContext::new(adapters.clone(), requested_default.clone());
            plugin.install(&mut context)?;
            for adapter in context.take_additions() {
                add_adapter(&mut adapters, adapter)?;
            }

            hooks.extend(plugin.hooks());
            middleware.extend(plugin.middleware());
        }

        let default_adapter = options
            .default_adapter
            .or_else(|| adapters.keys().next().cloned())
            .ok_or_else(|| {
                EmailSdkError::validation("create_email_client requires a default adapter.")
            })?;

        if !adapters.contains_key(&default_adapter) {
            return Err(EmailSdkError::provider_not_found(default_adapter));
        }

        hooks.extend(options.hooks);

        Ok(Self {
            adapters,
            default_adapter,
            fallback: options.fallback,
            retry: options.retry,
            hooks,
            middleware,
        })
    }

    pub fn adapters(&self) -> &HashMap<String, SharedEmailProvider> {
        &self.adapters
    }

    pub fn providers(&self) -> &HashMap<String, SharedEmailProvider> {
        &self.adapters
    }

    pub fn default_adapter(&self) -> &str {
        &self.default_adapter
    }

    pub fn default_provider(&self) -> &str {
        &self.default_adapter
    }

    pub fn adapter(&self, name: &str) -> Result<SharedEmailProvider, EmailSdkError> {
        self.adapters
            .get(name)
            .cloned()
            .ok_or_else(|| EmailSdkError::provider_not_found(name))
    }

    pub fn provider(&self, name: &str) -> Result<SharedEmailProvider, EmailSdkError> {
        self.adapter(name)
    }

    pub fn with_adapter(&self, name: impl Into<String>) -> Result<AdapterClient, EmailSdkError> {
        let name = name.into();
        self.adapter(&name)?;
        Ok(AdapterClient {
            client: self.clone(),
            adapter: name,
        })
    }

    pub fn with_provider(&self, name: impl Into<String>) -> Result<AdapterClient, EmailSdkError> {
        self.with_adapter(name)
    }

    pub async fn send(
        &self,
        message: EmailMessage,
        options: Option<SendOptions>,
    ) -> Result<EmailProviderResponse, EmailSdkError> {
        self.send_with_adapters(message, options).await
    }

    pub async fn send_batch(
        &self,
        messages: Vec<SendBatchItem>,
        options: Option<SendOptions>,
    ) -> Vec<SendBatchResult> {
        let mut results = Vec::with_capacity(messages.len());

        for (index, item) in messages.into_iter().enumerate() {
            let mut send_options = options.clone().unwrap_or_default();
            if let Some(adapter) = item.adapter {
                send_options.adapter = Some(adapter);
            }
            if let Some(fallback_adapters) = item.fallback_adapters {
                send_options.fallback_adapters = Some(fallback_adapters);
            }

            match self.send(item.message, Some(send_options)).await {
                Ok(response) => results.push(SendBatchResult::Ok { index, response }),
                Err(error) => results.push(SendBatchResult::Err { index, error }),
            }
        }

        results
    }

    async fn send_with_adapters(
        &self,
        message: EmailMessage,
        options: Option<SendOptions>,
    ) -> Result<EmailProviderResponse, EmailSdkError> {
        let prepared = self
            .apply_before_send_middleware(BeforeSendEvent { message, options })
            .await?;

        assert_message(&prepared.message)?;

        let adapter = prepared
            .options
            .as_ref()
            .and_then(|options| options.adapter.clone())
            .unwrap_or_else(|| self.default_adapter.clone());
        let fallback_adapters = prepared
            .options
            .as_ref()
            .and_then(|options| options.fallback_adapters.clone())
            .unwrap_or_else(|| self.fallback.clone());
        let adapter_names = resolve_adapter_order(adapter, fallback_adapters);
        let mut failures = Vec::new();

        for adapter_name in adapter_names {
            let provider = self.adapter(&adapter_name)?;

            match self
                .send_with_retry(
                    provider.clone(),
                    prepared.message.clone(),
                    prepared.options.clone(),
                )
                .await
            {
                Ok(response) => return Ok(response),
                Err(failure) => {
                    failures.push(failure.error.clone());
                    let event = SendFailureEvent {
                        provider: provider.name().to_owned(),
                        message: prepared.message.clone(),
                        attempt: failure.attempt,
                        metadata: prepared
                            .options
                            .as_ref()
                            .map(|options| options.metadata.clone())
                            .unwrap_or_default(),
                        error: failure.error.clone(),
                    };
                    self.invoke_error_middleware(event.clone()).await;
                    self.invoke_hooks_on_error(event).await;
                }
            }
        }

        if failures.len() == 1 {
            return Err(failures.remove(0));
        }

        Err(
            EmailSdkError::new("All email adapters failed.", "all_providers_failed")
                .details(format!("{failures:?}")),
        )
    }

    async fn send_with_retry(
        &self,
        provider: SharedEmailProvider,
        message: EmailMessage,
        options: Option<SendOptions>,
    ) -> Result<EmailProviderResponse, ProviderAttemptFailure> {
        let retries = options
            .as_ref()
            .and_then(|options| options.retries)
            .unwrap_or(self.retry.retries);

        for attempt in 1..=retries + 1 {
            let metadata = options
                .as_ref()
                .map(|options| options.metadata.clone())
                .unwrap_or_default();
            let hook_event = HookEvent {
                provider: provider.name().to_owned(),
                message: message.clone(),
                attempt,
                metadata: metadata.clone(),
            };
            self.invoke_hooks_before_send(hook_event.clone()).await;

            let context = EmailProviderContext {
                idempotency_key: options
                    .as_ref()
                    .and_then(|options| options.idempotency_key.clone())
                    .or_else(|| message.idempotency_key.clone()),
                attempt,
                metadata,
            };

            match provider.send(message.clone(), context).await {
                Ok(mut response) => {
                    if response.provider.is_empty() {
                        response.provider = provider.name().to_owned();
                    }

                    let success = SendSuccessEvent {
                        provider: provider.name().to_owned(),
                        message: message.clone(),
                        attempt,
                        metadata: options
                            .as_ref()
                            .map(|options| options.metadata.clone())
                            .unwrap_or_default(),
                        response: response.clone(),
                    };
                    self.invoke_after_send_middleware(success.clone()).await;
                    self.invoke_hooks_after_send(success).await;

                    return Ok(response);
                }
                Err(error) => {
                    let normalized = normalize_provider_error(provider.name(), error);
                    let can_retry =
                        attempt <= retries && (self.retry.should_retry)(&normalized, attempt);

                    if !can_retry {
                        return Err(ProviderAttemptFailure {
                            error: normalized,
                            attempt,
                        });
                    }

                    let delay = (self.retry.delay)(attempt, &normalized);
                    self.invoke_hooks_on_retry(
                        hook_event,
                        normalized,
                        attempt + 1,
                        delay.as_millis() as u64,
                    )
                    .await;
                    sleep_if_needed(delay).await;
                }
            }
        }

        Err(ProviderAttemptFailure {
            error: EmailSdkError::new("Email retry loop exited unexpectedly.", "retry_loop_exited"),
            attempt: retries + 1,
        })
    }

    async fn apply_before_send_middleware(
        &self,
        event: BeforeSendEvent,
    ) -> Result<BeforeSendEvent, EmailSdkError> {
        let mut message = event.message;
        let mut options = event.options;

        for item in &self.middleware {
            let result = item
                .before_send(BeforeSendEvent {
                    message: message.clone(),
                    options: options.clone(),
                })
                .await?;

            if let Some(result) = result {
                if let Some(next_message) = result.message {
                    message = next_message;
                }

                if let Some(next_options) = result.options {
                    options = Some(merge_options(options, next_options));
                }
            }
        }

        Ok(BeforeSendEvent { message, options })
    }

    async fn invoke_after_send_middleware(&self, event: SendSuccessEvent) {
        for item in &self.middleware {
            let _ = item.after_send(event.clone()).await;
        }
    }

    async fn invoke_error_middleware(&self, event: SendFailureEvent) {
        for item in &self.middleware {
            let _ = item.on_error(event.clone()).await;
        }
    }

    async fn invoke_hooks_before_send(&self, event: HookEvent) {
        for hooks in &self.hooks {
            let _ = hooks.before_send(event.clone()).await;
        }
    }

    async fn invoke_hooks_after_send(&self, event: SendSuccessEvent) {
        for hooks in &self.hooks {
            let _ = hooks
                .after_send(
                    HookEvent {
                        provider: event.provider.clone(),
                        message: event.message.clone(),
                        attempt: event.attempt,
                        metadata: event.metadata.clone(),
                    },
                    event.response.clone(),
                )
                .await;
        }
    }

    async fn invoke_hooks_on_error(&self, event: SendFailureEvent) {
        for hooks in &self.hooks {
            let _ = hooks
                .on_error(
                    HookEvent {
                        provider: event.provider.clone(),
                        message: event.message.clone(),
                        attempt: event.attempt,
                        metadata: event.metadata.clone(),
                    },
                    event.error.clone(),
                )
                .await;
        }
    }

    async fn invoke_hooks_on_retry(
        &self,
        event: HookEvent,
        error: EmailSdkError,
        next_attempt: usize,
        delay_ms: u64,
    ) {
        for hooks in &self.hooks {
            let _ = hooks
                .on_retry(event.clone(), error.clone(), next_attempt, delay_ms)
                .await;
        }
    }
}

#[derive(Clone)]
pub struct AdapterClient {
    client: EmailClient,
    adapter: String,
}

impl AdapterClient {
    pub async fn send(
        &self,
        message: EmailMessage,
        options: Option<SendOptions>,
    ) -> Result<EmailProviderResponse, EmailSdkError> {
        let mut options = options.unwrap_or_default();
        options.adapter = Some(self.adapter.clone());
        self.client.send(message, Some(options)).await
    }

    pub async fn send_batch(
        &self,
        messages: Vec<SendBatchItem>,
        options: Option<SendOptions>,
    ) -> Vec<SendBatchResult> {
        let mut options = options.unwrap_or_default();
        options.adapter = Some(self.adapter.clone());
        self.client.send_batch(messages, Some(options)).await
    }
}

#[derive(Debug)]
struct ProviderAttemptFailure {
    error: EmailSdkError,
    attempt: usize,
}

fn add_adapter(
    adapters: &mut HashMap<String, SharedEmailProvider>,
    adapter: SharedEmailProvider,
) -> Result<(), EmailSdkError> {
    let name = adapter.name().to_owned();
    if adapters.contains_key(&name) {
        return Err(EmailSdkError::validation(format!(
            "Duplicate email adapter \"{name}\"."
        )));
    }

    adapters.insert(name, adapter);
    Ok(())
}

fn resolve_adapter_order(adapter: String, fallback_adapters: Vec<String>) -> Vec<String> {
    let mut adapters = Vec::with_capacity(fallback_adapters.len() + 1);
    adapters.push(adapter);

    for fallback in fallback_adapters {
        if !adapters.contains(&fallback) {
            adapters.push(fallback);
        }
    }

    adapters
}

fn merge_options(current: Option<SendOptions>, next: SendOptions) -> SendOptions {
    let mut merged = current.unwrap_or_default();

    if next.adapter.is_some() {
        merged.adapter = next.adapter;
    }
    if next.fallback_adapters.is_some() {
        merged.fallback_adapters = next.fallback_adapters;
    }
    if next.retries.is_some() {
        merged.retries = next.retries;
    }
    if next.idempotency_key.is_some() {
        merged.idempotency_key = next.idempotency_key;
    }
    merged.metadata.extend(next.metadata);

    merged
}

fn normalize_provider_error(provider: &str, error: EmailSdkError) -> EmailSdkError {
    if error.provider.as_deref() == Some(provider) {
        return error;
    }

    EmailSdkError::provider_error(error.message.clone(), provider)
        .retryable(error.retryable)
        .details(error.details.unwrap_or_default())
}

async fn sleep_if_needed(delay: Duration) {
    if !delay.is_zero() {
        sleep(delay).await;
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use async_trait::async_trait;

    use super::*;
    use crate::{EmailProvider, EmailProviderContext};

    #[derive(Debug)]
    struct StaticProvider {
        name: String,
        sends: AtomicUsize,
    }

    impl StaticProvider {
        fn shared(name: &str) -> Arc<Self> {
            Arc::new(Self {
                name: name.to_owned(),
                sends: AtomicUsize::new(0),
            })
        }

        fn sends(&self) -> usize {
            self.sends.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl EmailProvider for StaticProvider {
        fn name(&self) -> &str {
            &self.name
        }

        async fn send(
            &self,
            _message: EmailMessage,
            _context: EmailProviderContext,
        ) -> Result<EmailProviderResponse, EmailSdkError> {
            let count = self.sends.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(EmailProviderResponse::new(self.name.clone())
                .id(format!("{}_{}", self.name, count))
                .message_id(format!("{}_{}", self.name, count)))
        }
    }

    #[derive(Debug)]
    struct FailProvider {
        name: String,
        attempts: AtomicUsize,
        fail_until: usize,
        retryable: bool,
    }

    impl FailProvider {
        fn shared(name: &str, fail_until: usize, retryable: bool) -> Arc<Self> {
            Arc::new(Self {
                name: name.to_owned(),
                attempts: AtomicUsize::new(0),
                fail_until,
                retryable,
            })
        }

        fn attempts(&self) -> usize {
            self.attempts.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl EmailProvider for FailProvider {
        fn name(&self) -> &str {
            &self.name
        }

        async fn send(
            &self,
            _message: EmailMessage,
            _context: EmailProviderContext,
        ) -> Result<EmailProviderResponse, EmailSdkError> {
            let attempt = self.attempts.fetch_add(1, Ordering::SeqCst) + 1;
            if attempt <= self.fail_until {
                return Err(EmailSdkError::provider_error("boom", self.name.clone())
                    .retryable(self.retryable));
            }

            Ok(EmailProviderResponse::new(self.name.clone())
                .id(format!("{}_{}", self.name, attempt)))
        }
    }

    struct AddAdapterPlugin {
        provider: Arc<StaticProvider>,
    }

    impl EmailPlugin for AddAdapterPlugin {
        fn id(&self) -> &str {
            "add-adapter"
        }

        fn install(&self, context: &mut EmailPluginContext) -> Result<(), EmailSdkError> {
            context.add_adapter(self.provider.clone());
            Ok(())
        }
    }

    struct SubjectMiddleware;

    #[async_trait]
    impl EmailSendMiddleware for SubjectMiddleware {
        async fn before_send(
            &self,
            event: BeforeSendEvent,
        ) -> Result<Option<crate::BeforeSendResult>, EmailSdkError> {
            let mut message = event.message;
            message.subject = format!("[tag] {}", message.subject);
            Ok(Some(crate::BeforeSendResult {
                message: Some(message),
                options: None,
            }))
        }
    }

    struct MiddlewarePlugin;

    impl EmailPlugin for MiddlewarePlugin {
        fn id(&self) -> &str {
            "middleware"
        }

        fn middleware(&self) -> Vec<Arc<dyn EmailSendMiddleware>> {
            vec![Arc::new(SubjectMiddleware)]
        }
    }

    struct CaptureProvider {
        name: String,
        subjects: Mutex<Vec<String>>,
    }

    impl CaptureProvider {
        fn shared(name: &str) -> Arc<Self> {
            Arc::new(Self {
                name: name.to_owned(),
                subjects: Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl EmailProvider for CaptureProvider {
        fn name(&self) -> &str {
            &self.name
        }

        async fn send(
            &self,
            message: EmailMessage,
            _context: EmailProviderContext,
        ) -> Result<EmailProviderResponse, EmailSdkError> {
            self.subjects.lock().unwrap().push(message.subject);
            Ok(EmailProviderResponse::new(self.name.clone()))
        }
    }

    fn message() -> EmailMessage {
        EmailMessage::builder("from@example.com", "to@example.com", "Hello")
            .text("Hello there")
            .build()
    }

    #[tokio::test]
    async fn sends_with_default_adapter() {
        let provider = StaticProvider::shared("memory");
        let client =
            create_email_client(EmailClientOptions::new().adapter(provider.clone())).unwrap();

        let response = client.send(message(), None).await.unwrap();

        assert_eq!(response.provider, "memory");
        assert_eq!(provider.sends(), 1);
    }

    #[tokio::test]
    async fn fails_validation_before_provider_send() {
        let provider = StaticProvider::shared("memory");
        let client =
            create_email_client(EmailClientOptions::new().adapter(provider.clone())).unwrap();
        let invalid = EmailMessage::builder("from@example.com", "to@example.com", "Hello").build();

        let error = client.send(invalid, None).await.unwrap_err();

        assert_eq!(error.code, "validation_error");
        assert_eq!(provider.sends(), 0);
    }

    #[tokio::test]
    async fn falls_back_after_provider_failure() {
        let primary = FailProvider::shared("primary", usize::MAX, false);
        let backup = StaticProvider::shared("backup");
        let client = create_email_client(
            EmailClientOptions::new()
                .adapter(primary.clone())
                .adapter(backup.clone())
                .default_adapter("primary")
                .fallback(["backup"]),
        )
        .unwrap();

        let response = client.send(message(), None).await.unwrap();

        assert_eq!(response.provider, "backup");
        assert_eq!(primary.attempts(), 1);
        assert_eq!(backup.sends(), 1);
    }

    #[tokio::test]
    async fn retries_retryable_provider_errors() {
        let provider = FailProvider::shared("primary", 1, true);
        let retry = EmailRetryConfig {
            retries: 1,
            delay: Arc::new(|_, _| Duration::from_millis(0)),
            should_retry: Arc::new(|error, _| error.retryable),
        };
        let client = create_email_client(
            EmailClientOptions::new()
                .adapter(provider.clone())
                .retry(retry),
        )
        .unwrap();

        let response = client.send(message(), None).await.unwrap();

        assert_eq!(response.provider, "primary");
        assert_eq!(provider.attempts(), 2);
    }

    #[tokio::test]
    async fn plugin_can_add_adapter() {
        let provider = StaticProvider::shared("plugin");
        let client =
            create_email_client(EmailClientOptions::new().default_adapter("plugin").plugin(
                Arc::new(AddAdapterPlugin {
                    provider: provider.clone(),
                }),
            ))
            .unwrap();

        let response = client.send(message(), None).await.unwrap();

        assert_eq!(response.provider, "plugin");
        assert_eq!(provider.sends(), 1);
    }

    #[tokio::test]
    async fn plugin_middleware_can_mutate_message() {
        let provider = CaptureProvider::shared("capture");
        let client = create_email_client(
            EmailClientOptions::new()
                .adapter(provider.clone())
                .plugin(Arc::new(MiddlewarePlugin)),
        )
        .unwrap();

        client.send(message(), None).await.unwrap();

        assert_eq!(
            provider.subjects.lock().unwrap().as_slice(),
            ["[tag] Hello"]
        );
    }

    #[tokio::test]
    async fn send_batch_returns_per_message_results() {
        let provider = StaticProvider::shared("memory");
        let client =
            create_email_client(EmailClientOptions::new().adapter(provider.clone())).unwrap();

        let results = client
            .send_batch(
                vec![
                    SendBatchItem::new(message()),
                    SendBatchItem::new(
                        EmailMessage::builder("from@example.com", "to@example.com", "Bad").build(),
                    ),
                ],
                None,
            )
            .await;

        assert!(matches!(results[0], SendBatchResult::Ok { index: 0, .. }));
        assert!(matches!(results[1], SendBatchResult::Err { index: 1, .. }));
        assert_eq!(provider.sends(), 1);
    }
}
