//! Pure strategy code: the TWAP model, book walking and fees, and strategy v0. No I/O and no
//! clock, so replay, shadow, paper and live run the same functions.

pub mod fill;
pub mod model;
pub mod v0;

use sha2::{Digest, Sha256};

/// A short, stable id of a parameter set: the first 16 hex digits of SHA-256 over its JSON.
pub fn params_hash(params: &serde_json::Value) -> String {
    let digest = Sha256::digest(params.to_string().as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_hash_is_stable_and_sensitive() {
        let a = serde_json::json!({"model": model::ModelParams::default(), "v0": v0::V0Params::default()});
        assert_eq!(params_hash(&a), params_hash(&a.clone()));
        assert_eq!(params_hash(&a).len(), 16);
        let mut b = a.clone();
        b["v0"]["min_edge"] = serde_json::json!("0.02");
        assert_ne!(params_hash(&a), params_hash(&b));
    }
}
