//! Pipeline errors. The host maps these; they carry no localized text.

use std::fmt;

#[derive(Debug)]
pub enum GradeError {
    InvalidDocument(String),
    TypeMismatch {
        node: String,
        input: String,
    },
    Cycle {
        nodes: Vec<String>,
    },
    MissingCameraMatrix,
    SizeMismatch {
        node: String,
    },
    Io(String),
    Cancelled,
    /// Stable machine code. The host translates it; the code is not a sentence.
    Rejected(&'static str),
}

impl fmt::Display for GradeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDocument(message) => write!(f, "{message}"),
            Self::TypeMismatch { node, input } => {
                write!(f, "type mismatch on {node} input {input}")
            }
            Self::Cycle { nodes } => write!(f, "cycle: {}", nodes.join(", ")),
            Self::MissingCameraMatrix => write!(f, "missing camera matrix"),
            Self::SizeMismatch { node } => write!(f, "size mismatch at {node}"),
            Self::Io(message) => write!(f, "{message}"),
            Self::Cancelled => write!(f, "cancelled"),
            Self::Rejected(code) => write!(f, "{code}"),
        }
    }
}

impl std::error::Error for GradeError {}
