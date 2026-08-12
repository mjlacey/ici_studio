use thiserror::Error;

#[derive(Debug, Error)]
pub enum MdfError {
    #[error("Buffer too small at {file}:{line}: need at least {expected} bytes, got {actual}")]
    TooShortBuffer {
        actual:   usize,
        expected: usize,
        file:     &'static str,
        line:     u32,
    },

    #[error(r#"Invalid file identifier: Expected "MDF     ", found {0}"#)]
    FileIdentifierError(String),

    #[error(r#"File version too low: Expected "> 4.1", found {0}"#)]
    FileVersioningError(String),

    #[error("Invalid block identifier: Expected {expected:?}, got {actual:?}")]
    BlockIDError {
        actual: String,
        expected: String,
    },

    #[error("I/O error: {0}")]
    IOError(#[from] std::io::Error),

    #[error("Invalid version string: {0}")]
    InvalidVersionString(String),

    #[error("Block linking error: {0}")]
    BlockLinkError(String),

    #[error("Block serialization error: {0}")]
    BlockSerializationError(String),

    #[error("Conversion chain too deep: maximum depth of {max_depth} exceeded")]
    ConversionChainTooDeep { max_depth: usize },

    #[error("Conversion chain cycle detected at block address {address:#x}")]
    ConversionChainCycle { address: u64 },
}
