use axum::extract::Path;
use axum::Json;
use serde::Serialize;

use claria_instruments::scoring::Domain;
use claria_instruments::{all_instruments, get_instrument};

use crate::error::ApiError;

#[derive(Serialize)]
pub struct InstrumentSummary {
    id: String,
    name: String,
}

#[derive(Serialize)]
pub struct InstrumentDetail {
    id: String,
    name: String,
    domains: Vec<Domain>,
}

pub async fn list_instruments() -> Json<Vec<InstrumentSummary>> {
    let instruments: Vec<InstrumentSummary> = all_instruments()
        .iter()
        .map(|i| InstrumentSummary {
            id: i.id().to_string(),
            name: i.name().to_string(),
        })
        .collect();
    Json(instruments)
}

pub async fn get_instrument_detail(
    Path(id): Path<String>,
) -> Result<Json<InstrumentDetail>, ApiError> {
    let instrument = get_instrument(&id)
        .ok_or_else(|| ApiError::NotFound(format!("instrument not found: {id}")))?;

    Ok(Json(InstrumentDetail {
        id: instrument.id().to_string(),
        name: instrument.name().to_string(),
        domains: instrument.domains().to_vec(),
    }))
}
