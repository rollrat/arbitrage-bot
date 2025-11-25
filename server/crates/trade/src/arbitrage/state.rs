use chrono::{DateTime, Utc};
use interface::ExchangeError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

use crate::trader::binance::HedgedPair;

const STATE_FILE: &str = "arb_state.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbitrageState {
    pub open: bool,
    pub dir: Option<String>, // "carry" or "reverse"
    pub pair: HedgedPair,
    pub symbol: String,
    pub last_open_basis_bps: Option<f64>,
    pub last_close_basis_bps: Option<f64>,
    pub actions: Option<serde_json::Value>,
    pub updated_at: DateTime<Utc>,
}

impl Default for ArbitrageState {
    fn default() -> Self {
        Self {
            open: false,
            dir: None,
            pair: HedgedPair::default(),
            symbol: "BTCUSDT".to_string(),
            last_open_basis_bps: None,
            last_close_basis_bps: None,
            actions: None,
            updated_at: Utc::now(),
        }
    }
}

impl ArbitrageState {
    pub fn new(symbol: String) -> Self {
        Self {
            symbol,
            ..Default::default()
        }
    }

    pub fn read() -> Result<Self, ExchangeError> {
        if !Path::new(STATE_FILE).exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(STATE_FILE)
            .map_err(|e| ExchangeError::Other(format!("Failed to read state file: {}", e)))?;

        let state: ArbitrageState = serde_json::from_str(&content)
            .map_err(|e| ExchangeError::Other(format!("Failed to parse state file: {}", e)))?;

        Ok(state)
    }

    pub fn write(&self) -> Result<(), ExchangeError> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| ExchangeError::Other(format!("Failed to serialize state: {}", e)))?;

        fs::write(STATE_FILE, content)
            .map_err(|e| ExchangeError::Other(format!("Failed to write state file: {}", e)))?;

        Ok(())
    }

    pub fn update_position(
        &mut self,
        open: bool,
        dir: Option<String>,
        pair: HedgedPair,
        basis_bps: Option<f64>,
        actions: Option<serde_json::Value>,
    ) {
        self.open = open;
        self.dir = dir.clone();
        self.pair = pair;
        self.updated_at = Utc::now();

        if open {
            self.last_open_basis_bps = basis_bps;
        } else {
            self.last_close_basis_bps = basis_bps;
        }

        self.actions = actions;
    }
}
