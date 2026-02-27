use thiserror::Error;

use crate::scoring::ValidationError;

#[derive(Debug, Error)]
pub enum InstrumentError {
    #[error("unknown instrument: {0}")]
    UnknownInstrument(String),

    #[error("validation failed: {0}")]
    Validation(#[from] ValidationError),

    #[error("unknown subscale '{subscale_id}' for instrument '{instrument_id}'")]
    UnknownSubscale {
        instrument_id: String,
        subscale_id: String,
    },
}
