use crate::config;

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum ExpandedFormKind {
    LocalProvider,
    LocalModel,
    DocsInit,
    InitInstructions,
}

impl ExpandedFormKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalProvider => "local_provider",
            Self::LocalModel => "local_model",
            Self::DocsInit => "docs_init",
            Self::InitInstructions => "init_instructions",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::LocalProvider => "Add Local LLM Provider",
            Self::LocalModel => "Add Local Model",
            Self::DocsInit => "Docs Init Draft",
            Self::InitInstructions => "Project Init Draft",
        }
    }
}

pub struct ExpandedFormField {
    pub key: &'static str,
    pub label: &'static str,
    pub value: String,
    pub required: bool,
    pub secret: bool,
}

pub struct ExpandedFormState {
    pub open: bool,
    pub kind: ExpandedFormKind,
    pub fields: Vec<ExpandedFormField>,
    pub focused: usize,
    pub validation_message: Option<String>,
}

impl Default for ExpandedFormState {
    fn default() -> Self {
        Self {
            open: false,
            kind: ExpandedFormKind::LocalProvider,
            fields: Vec::new(),
            focused: 0,
            validation_message: None,
        }
    }
}

impl ExpandedFormState {
    pub fn open(&mut self, kind: ExpandedFormKind) -> ExpandedFormEvents {
        self.open = true;
        self.kind = kind;
        self.fields = fields_for(kind);
        self.focused = 0;
        self.validation_message = None;
        ExpandedFormEvents::single(ExpandedFormEvent::Opened { kind })
    }

    pub fn cancel(&mut self) -> ExpandedFormEvents {
        if !self.open {
            return ExpandedFormEvents::none();
        }

        let kind = self.kind;
        self.close();
        ExpandedFormEvents::single(ExpandedFormEvent::Cancelled { kind })
    }

    pub fn focus_next(&mut self) {
        if self.fields.is_empty() {
            self.focused = 0;
            return;
        }

        self.focused = (self.focused + 1) % self.fields.len();
        self.validation_message = None;
    }

    pub fn focus_previous(&mut self) {
        if self.fields.is_empty() {
            self.focused = 0;
            return;
        }

        if self.focused == 0 {
            self.focused = self.fields.len() - 1;
        } else {
            self.focused -= 1;
        }
        self.validation_message = None;
    }

    pub fn push_char(&mut self, value: char) -> ExpandedFormEvents {
        let Some(field) = self.fields.get_mut(self.focused) else {
            return ExpandedFormEvents::none();
        };

        field.value.push(value);
        self.validation_message = None;
        ExpandedFormEvents::single(ExpandedFormEvent::FieldChanged {
            kind: self.kind,
            field: field.key,
            masked: field.secret,
        })
    }

    pub fn backspace(&mut self) -> ExpandedFormEvents {
        let Some(field) = self.fields.get_mut(self.focused) else {
            return ExpandedFormEvents::none();
        };

        if field.value.pop().is_none() {
            return ExpandedFormEvents::none();
        }

        self.validation_message = None;
        ExpandedFormEvents::single(ExpandedFormEvent::FieldChanged {
            kind: self.kind,
            field: field.key,
            masked: field.secret,
        })
    }

    pub fn submit(&mut self) -> ExpandedFormSubmit {
        if let Some(message) = self.validate() {
            self.validation_message = Some(message);
            return ExpandedFormSubmit {
                submitted: false,
                events: ExpandedFormEvents::none(),
                notice: None,
            };
        }

        let kind = self.kind;
        self.close();
        ExpandedFormSubmit {
            submitted: true,
            events: ExpandedFormEvents::single(ExpandedFormEvent::Submitted { kind }),
            notice: Some(format!("expanded form submitted: {}", kind.as_str())),
        }
    }

    fn close(&mut self) {
        self.open = false;
        self.fields.clear();
        self.focused = 0;
        self.validation_message = None;
    }

    fn validate(&self) -> Option<String> {
        for field in &self.fields {
            if field.required && field.value.trim().is_empty() {
                return Some(format!("{} is required", field.label));
            }
        }

        if let Some(context) = self
            .fields
            .iter()
            .find(|field| field.key == "context_tokens")
        {
            let Ok(value) = context.value.trim().parse::<u32>() else {
                return Some("context tokens must be a number".to_owned());
            };
            if value == 0 {
                return Some("context tokens must be greater than 0".to_owned());
            }
        }

        None
    }
}

pub struct ExpandedFormSubmit {
    pub submitted: bool,
    pub events: ExpandedFormEvents,
    pub notice: Option<String>,
}

pub enum ExpandedFormEvent {
    Opened {
        kind: ExpandedFormKind,
    },
    FieldChanged {
        kind: ExpandedFormKind,
        field: &'static str,
        masked: bool,
    },
    Submitted {
        kind: ExpandedFormKind,
    },
    Cancelled {
        kind: ExpandedFormKind,
    },
}

#[derive(Default)]
pub struct ExpandedFormEvents {
    pub events: Vec<ExpandedFormEvent>,
}

impl ExpandedFormEvents {
    pub fn none() -> Self {
        Self { events: Vec::new() }
    }

    pub fn single(event: ExpandedFormEvent) -> Self {
        Self {
            events: vec![event],
        }
    }
}

fn fields_for(kind: ExpandedFormKind) -> Vec<ExpandedFormField> {
    match kind {
        ExpandedFormKind::LocalProvider => vec![
            field("provider_name", "provider name", config::DEFAULT_PROVIDER),
            field("base_url", "base url", config::DEFAULT_BASE_URL),
            field("model", "model", config::DEFAULT_MODEL),
            field(
                "context_tokens",
                "context tokens",
                config::DEFAULT_CONTEXT_TOKENS.to_string(),
            ),
            optional_field("api_key_env", "api key env", ""),
        ],
        ExpandedFormKind::LocalModel => vec![
            field("provider_name", "provider name", config::DEFAULT_PROVIDER),
            field("model", "model", config::DEFAULT_MODEL),
            field(
                "context_tokens",
                "context tokens",
                config::DEFAULT_CONTEXT_TOKENS.to_string(),
            ),
        ],
        ExpandedFormKind::DocsInit => vec![
            field("guide_file", "guide file", "documentation-guide.md"),
            field("template_dir", "template dir", "templates"),
            field("router_file", "router file", "docs/PROJECT-CONTEXT.md"),
            field("use_templates", "use templates", "true"),
        ],
        ExpandedFormKind::InitInstructions => {
            vec![field("project_file", "project file", "AGENTS.md")]
        }
    }
}

fn field(key: &'static str, label: &'static str, value: impl Into<String>) -> ExpandedFormField {
    ExpandedFormField {
        key,
        label,
        value: value.into(),
        required: true,
        secret: false,
    }
}

fn optional_field(
    key: &'static str,
    label: &'static str,
    value: impl Into<String>,
) -> ExpandedFormField {
    ExpandedFormField {
        key,
        label,
        value: value.into(),
        required: false,
        secret: true,
    }
}

#[cfg(test)]
mod tests {
    use super::{ExpandedFormKind, ExpandedFormState};
    use crate::config;

    #[test]
    fn local_provider_form_uses_config_defaults() {
        let mut form = ExpandedFormState::default();

        form.open(ExpandedFormKind::LocalProvider);

        assert_eq!(
            field_value(&form, "provider_name"),
            config::DEFAULT_PROVIDER
        );
        assert_eq!(field_value(&form, "base_url"), config::DEFAULT_BASE_URL);
        assert_eq!(field_value(&form, "model"), config::DEFAULT_MODEL);
        assert_eq!(
            field_value(&form, "context_tokens"),
            config::DEFAULT_CONTEXT_TOKENS.to_string()
        );
    }

    #[test]
    fn local_model_form_uses_config_defaults() {
        let mut form = ExpandedFormState::default();

        form.open(ExpandedFormKind::LocalModel);

        assert_eq!(
            field_value(&form, "provider_name"),
            config::DEFAULT_PROVIDER
        );
        assert_eq!(field_value(&form, "model"), config::DEFAULT_MODEL);
        assert_eq!(
            field_value(&form, "context_tokens"),
            config::DEFAULT_CONTEXT_TOKENS.to_string()
        );
    }

    fn field_value(form: &ExpandedFormState, key: &str) -> String {
        form.fields
            .iter()
            .find(|field| field.key == key)
            .map(|field| field.value.clone())
            .unwrap_or_else(|| panic!("missing field {key}"))
    }
}
