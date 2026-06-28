use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmailAddress {
    Address(String),
    Named { email: String, name: String },
}

impl EmailAddress {
    pub fn new(email: impl Into<String>) -> Self {
        Self::Address(email.into())
    }

    pub fn named(email: impl Into<String>, name: impl Into<String>) -> Self {
        Self::Named {
            email: email.into(),
            name: name.into(),
        }
    }

    pub fn email(&self) -> &str {
        match self {
            Self::Address(email) => email,
            Self::Named { email, .. } => email,
        }
    }

    pub fn formatted(&self) -> String {
        match self {
            Self::Address(email) => email.clone(),
            Self::Named { email, name } => format!("{name} <{email}>"),
        }
    }
}

impl From<&str> for EmailAddress {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for EmailAddress {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailAttachment {
    pub filename: String,
    pub content: Option<Vec<u8>>,
    pub path: Option<String>,
    pub content_type: Option<String>,
    pub content_id: Option<String>,
    pub disposition: Option<EmailAttachmentDisposition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailAttachmentDisposition {
    Attachment,
    Inline,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Headers {
    Map(BTreeMap<String, String>),
    List(Vec<EmailHeader>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailTag {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MetadataValue {
    String(String),
    Number(f64),
    Bool(bool),
    Null,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmailMessage {
    pub from: EmailAddress,
    pub to: Vec<EmailAddress>,
    pub subject: String,
    pub html: Option<String>,
    pub text: Option<String>,
    pub cc: Vec<EmailAddress>,
    pub bcc: Vec<EmailAddress>,
    pub reply_to: Vec<EmailAddress>,
    pub headers: Option<Headers>,
    pub attachments: Vec<EmailAttachment>,
    pub tags: Vec<EmailTag>,
    pub metadata: BTreeMap<String, MetadataValue>,
    pub idempotency_key: Option<String>,
}

impl EmailMessage {
    pub fn builder(
        from: impl Into<EmailAddress>,
        to: impl Into<EmailAddress>,
        subject: impl Into<String>,
    ) -> EmailMessageBuilder {
        EmailMessageBuilder::new(from, to, subject)
    }
}

#[derive(Debug, Clone)]
pub struct EmailMessageBuilder {
    message: EmailMessage,
}

impl EmailMessageBuilder {
    pub fn new(
        from: impl Into<EmailAddress>,
        to: impl Into<EmailAddress>,
        subject: impl Into<String>,
    ) -> Self {
        Self {
            message: EmailMessage {
                from: from.into(),
                to: vec![to.into()],
                subject: subject.into(),
                html: None,
                text: None,
                cc: Vec::new(),
                bcc: Vec::new(),
                reply_to: Vec::new(),
                headers: None,
                attachments: Vec::new(),
                tags: Vec::new(),
                metadata: BTreeMap::new(),
                idempotency_key: None,
            },
        }
    }

    pub fn to(mut self, address: impl Into<EmailAddress>) -> Self {
        self.message.to.push(address.into());
        self
    }

    pub fn html(mut self, html: impl Into<String>) -> Self {
        self.message.html = Some(html.into());
        self
    }

    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.message.text = Some(text.into());
        self
    }

    pub fn cc(mut self, address: impl Into<EmailAddress>) -> Self {
        self.message.cc.push(address.into());
        self
    }

    pub fn bcc(mut self, address: impl Into<EmailAddress>) -> Self {
        self.message.bcc.push(address.into());
        self
    }

    pub fn reply_to(mut self, address: impl Into<EmailAddress>) -> Self {
        self.message.reply_to.push(address.into());
        self
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        match &mut self.message.headers {
            Some(Headers::List(headers)) => headers.push(EmailHeader {
                name: name.into(),
                value: value.into(),
            }),
            Some(Headers::Map(headers)) => {
                headers.insert(name.into(), value.into());
            }
            None => {
                self.message.headers = Some(Headers::List(vec![EmailHeader {
                    name: name.into(),
                    value: value.into(),
                }]));
            }
        }
        self
    }

    pub fn attachment(mut self, attachment: EmailAttachment) -> Self {
        self.message.attachments.push(attachment);
        self
    }

    pub fn tag(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.message.tags.push(EmailTag {
            name: name.into(),
            value: value.into(),
        });
        self
    }

    pub fn metadata(mut self, key: impl Into<String>, value: MetadataValue) -> Self {
        self.message.metadata.insert(key.into(), value);
        self
    }

    pub fn idempotency_key(mut self, value: impl Into<String>) -> Self {
        self.message.idempotency_key = Some(value.into());
        self
    }

    pub fn build(self) -> EmailMessage {
        self.message
    }
}
