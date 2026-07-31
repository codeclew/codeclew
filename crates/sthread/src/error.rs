use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    UnsupportedKotlinVersion,
    UnsupportedProjectConfiguration,
    ProjectModelChanged,
    WorkerProtocolMismatch,
    WorkerCrashed,
    SymbolNotFound,
    AmbiguousSymbol,
    ExpressionNotFound,
    StaleTarget,
    AmbiguousTarget,
    PreconditionFailed,
    UnsupportedControlFlow,
    IncompleteSemanticAnalysis,
    SliceBudgetExceeded,
    ReplacementParseError,
    TypeMismatch,
    BindingChanged,
    NewDiagnostics,
    EffectChanged,
    WritesetExceeded,
    CompileFailed,
    TestFailed,
    AbiChanged,
    RwConflict,
    WwConflict,
    StaleRequiresReslice,
    RefCompareAndSwapFailed,
    TransactionRecoveryRequired,
    InvalidInput,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize, Error)]
#[error("{code:?}: {message}")]
pub struct SthreadError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
    pub retryable: bool,
}

impl SthreadError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            transaction_id: None,
            snapshot_id: None,
            evidence: vec![],
            retryable: false,
        }
    }
}
