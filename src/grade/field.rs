//! Parameter widgets the grade UI reads from a node.

#[derive(Clone, Debug)]
pub struct FieldSpec {
    pub key: String,
    pub label: String,
    pub kind: FieldKind,
    pub value: f32,
    pub min: f32,
    pub max: f32,
    pub choice: i32,
    pub choices: String,
    pub display: String,
    pub n_options: i32,
    pub opt0: String,
    pub opt1: String,
    pub opt2: String,
    pub opt3: String,
    /// Pipe-separated wire values for a choice field. Empty otherwise.
    pub wires: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Slider,
    Choice,
    Toggle,
    Readonly,
}

impl FieldKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Slider => "slider",
            Self::Choice => "choice",
            Self::Toggle => "toggle",
            Self::Readonly => "readonly",
        }
    }
}

pub(crate) fn slider(
    key: &str,
    label: impl Into<String>,
    value: f32,
    min: f32,
    max: f32,
    display: String,
) -> FieldSpec {
    FieldSpec {
        key: key.into(),
        label: label.into(),
        kind: FieldKind::Slider,
        value,
        min,
        max,
        choice: 0,
        choices: String::new(),
        display,
        n_options: 0,
        opt0: String::new(),
        opt1: String::new(),
        opt2: String::new(),
        opt3: String::new(),
        wires: String::new(),
    }
}

pub(crate) fn choice(
    key: &str,
    label: String,
    choice: i32,
    values: &[&str],
    zh: bool,
    zh_labels: &[&str],
) -> FieldSpec {
    let labels: Vec<String> = if zh {
        zh_labels.iter().map(|label| (*label).to_string()).collect()
    } else {
        values
            .iter()
            .map(|value| value.split('|').next().unwrap_or(value).to_string())
            .collect()
    };
    let display = labels.get(choice as usize).cloned().unwrap_or_default();
    let opt = |index: usize| labels.get(index).cloned().unwrap_or_default();
    FieldSpec {
        key: key.into(),
        label,
        kind: FieldKind::Choice,
        value: choice as f32,
        min: 0.0,
        max: (values.len() as i32 - 1) as f32,
        choice,
        choices: labels.join("|"),
        display,
        n_options: labels.len() as i32,
        opt0: opt(0),
        opt1: opt(1),
        opt2: opt(2),
        opt3: opt(3),
        wires: values
            .iter()
            .map(|value| value.split_once('|').map(|(_, wire)| wire).unwrap_or(value))
            .collect::<Vec<_>>()
            .join("|"),
    }
}

pub(crate) fn toggle(
    key: &str,
    label: impl Into<String>,
    on: bool,
    on_label: impl Into<String>,
    off_label: impl Into<String>,
) -> FieldSpec {
    FieldSpec {
        key: key.into(),
        label: label.into(),
        kind: FieldKind::Toggle,
        value: if on { 1.0 } else { 0.0 },
        min: 0.0,
        max: 1.0,
        choice: 0,
        choices: String::new(),
        display: if on {
            on_label.into()
        } else {
            off_label.into()
        },
        n_options: 0,
        opt0: String::new(),
        opt1: String::new(),
        opt2: String::new(),
        opt3: String::new(),
        wires: String::new(),
    }
}

pub(crate) fn readonly(
    key: &str,
    label: impl Into<String>,
    display: impl Into<String>,
) -> FieldSpec {
    FieldSpec {
        key: key.into(),
        label: label.into(),
        kind: FieldKind::Readonly,
        value: 0.0,
        min: 0.0,
        max: 1.0,
        choice: 0,
        choices: String::new(),
        display: display.into(),
        n_options: 0,
        opt0: String::new(),
        opt1: String::new(),
        opt2: String::new(),
        opt3: String::new(),
        wires: String::new(),
    }
}
