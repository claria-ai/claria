use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TokenCount {
    pub input: u64,
    pub output: u64,
}

impl TokenCount {
    pub fn total(&self) -> u64 {
        self.input + self.output
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TokenUsage {
    pub tokens: TokenCount,
    pub cost_usd: f64,
}
