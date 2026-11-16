//! Proven meter accounting and the Rollup wire protocol.
use serde::{Deserialize, Serialize};

pub mod accounting;
pub mod transition;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedAction {
    pub message: String,
    pub signature: String,
}
