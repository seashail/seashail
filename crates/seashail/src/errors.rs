use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// A structured error suitable for returning to an MCP client as tool output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolError {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Value::is_null", default)]
    pub data: Value,
}

impl ToolError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: Value::Null,
        }
    }
}

#[derive(Debug, Error, Clone)]
pub enum SeashailError {
    #[error("wallet not found: {0}")]
    WalletNotFound(String),

    #[error("account index out of range")]
    AccountIndexOutOfRange,

    #[error("passphrase required")]
    PassphraseRequired,

    #[error("user declined")]
    UserDeclined,

    #[error("backup not confirmed")]
    BackupNotConfirmed,

    #[error("keystore busy")]
    KeystoreBusy,
    // Add more structured errors as we expand the policy engine + adapters.
}

impl From<SeashailError> for ToolError {
    fn from(e: SeashailError) -> Self {
        match e {
            SeashailError::WalletNotFound(name) => {
                Self::new("wallet_not_found", format!("wallet not found: {name}"))
            }
            SeashailError::AccountIndexOutOfRange => {
                Self::new("account_index_out_of_range", "account index out of range")
            }
            SeashailError::PassphraseRequired => {
                Self::new("passphrase_required", "passphrase required")
            }
            SeashailError::UserDeclined => Self::new("user_declined", "user declined"),
            SeashailError::BackupNotConfirmed => {
                Self::new("backup_not_confirmed", "backup not confirmed correctly")
            }
            SeashailError::KeystoreBusy => {
                Self::new("keystore_busy", "keystore busy; retry the operation")
            }
        }
    }
}
