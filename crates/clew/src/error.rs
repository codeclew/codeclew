use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    UnsupportedKotlinVersion,
    UnsupportedCompilerPluginAbi,
    UnsupportedProjectConfiguration,
    ProjectModelChanged,
    WorkerProtocolMismatch,
    WorkerPreparationRequired,
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
    WorktreeRecoveryRequired,
    InvalidInput,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize, Error)]
#[error("{code:?}: {message}")]
#[serde(rename_all = "camelCase")]
pub struct ClewError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "relevant_is_empty")]
    pub relevant_anchors_or_symbols: Box<[String]>,
    pub retryable: bool,
}

fn relevant_is_empty(values: &[String]) -> bool {
    values.is_empty()
}

impl ClewError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        let retryable = matches!(
            code,
            ErrorCode::WorkerCrashed
                | ErrorCode::RefCompareAndSwapFailed
                | ErrorCode::TransactionRecoveryRequired
                | ErrorCode::WorktreeRecoveryRequired
        );
        Self {
            code,
            message: message.into(),
            transaction_id: None,
            snapshot_id: None,
            evidence: vec![],
            relevant_anchors_or_symbols: Box::default(),
            retryable,
        }
    }

    pub fn with_transaction(mut self, transaction_id: impl Into<String>) -> Self {
        self.transaction_id = Some(transaction_id.into());
        self
    }

    pub fn with_snapshot(mut self, snapshot_id: impl Into<String>) -> Self {
        self.snapshot_id = Some(snapshot_id.into());
        self
    }

    pub fn with_relevant(mut self, value: impl Into<String>) -> Self {
        let value = value.into();
        if !value.is_empty() && !self.relevant_anchors_or_symbols.contains(&value) {
            let mut relevant = self.relevant_anchors_or_symbols.into_vec();
            relevant.push(value);
            self.relevant_anchors_or_symbols = relevant.into_boxed_slice();
        }
        self
    }
}
