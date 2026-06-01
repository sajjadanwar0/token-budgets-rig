//! # token-budgets-rig
//!
//! An N=1 deployment of the affine-typed [`token_budgets`] crate into
//! [Rig](https://crates.io/crates/rig-core), a production Rust LLM-agent
//! framework, demonstrating a compile-time-anchored, runtime-enforced dollar
//! cap on a real multi-agent delegation workload.
//!
//! ## Design
//! A session holds one [`BudgetPool`] whose `cap` is the hard dollar ceiling.
//! Every (sub-)agent call goes through [`prompt_budgeted`], which:
//! 1. estimates the *worst-case* cost (input estimate + a `max_output_tok`
//!    bound) and [`reserve`](BudgetPool::reserve)s it from the pool — a
//!    *pre-flight* refusal if the pool cannot cover it, so the cap is respected
//!    before the spend rather than detected after it;
//! 2. runs the actual completion (passed as a closure, so this library does not
//!    bind to Rig's exact API surface);
//! 3. reconciles the *actual* cost via [`Reservation::commit`], returning the
//!    unspent remainder to the pool.
//!
//! Because `Reservation` is move-only and the pool re-checks availability on
//! every reserve, no sub-agent — however buggy or adversarial — can push
//! cumulative session spend past `cap`. That is the multi-agent
//! non-bypassability property, demonstrated against a real framework rather
//! than in isolation.

use std::future::Future;
use token_budgets::{BudgetError, BudgetPool, AnthropicEstimator, ByteLength, TokenEstimator};

/// Per-token pricing in micro-cents (the `token_budgets` atomic unit).
/// 1 micro-cent = 1e-6 cent = 1e-8 USD.
#[derive(Clone, Copy, Debug)]
pub struct Pricing {
    pub input_uc_per_tok: u64,
    pub output_uc_per_tok: u64,
}

impl Pricing {
    /// Build from per-million-token USD rates, e.g. `per_million_usd(3.0, 15.0)`
    /// for a $3-in / $15-out model.
    pub fn per_million_usd(input_usd: f64, output_usd: f64) -> Self {
        // (USD per 1e6 tok) -> (micro-cents per tok):  * 1e8 (USD->uc) / 1e6  =  * 100
        Self {
            input_uc_per_tok: (input_usd * 100.0).round() as u64,
            output_uc_per_tok: (output_usd * 100.0).round() as u64,
        }
    }

    /// Cost in micro-cents for a call of `in_tok` input and `out_tok` output tokens.
    #[inline]
    pub fn cost_uc(&self, in_tok: u64, out_tok: u64) -> u64 {
        in_tok
            .saturating_mul(self.input_uc_per_tok)
            .saturating_add(out_tok.saturating_mul(self.output_uc_per_tok))
    }
}

/// Convert a USD amount to the micro-cent budget unit. `$0.50` -> `50_000_000`.
pub fn usd_to_uc(usd: f64) -> u64 {
    (usd * 100.0 * 1_000_000.0).round() as u64
}

/// Failure modes of a budgeted call.
#[derive(Debug)]
pub enum BudgetedError {
    /// Pre-flight refusal: the pool cannot cover the worst-case reservation,
    /// i.e. honouring this call would breach the session cap.
    Reserve(BudgetError),
    /// The actual cost exceeded the reservation (the estimator under-counted).
    Commit(BudgetError),
    /// The underlying LLM call itself failed.
    Call(anyhow::Error),
}

impl std::fmt::Display for BudgetedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BudgetedError::Reserve(e) => {
                write!(f, "budget reservation refused (cap would be exceeded): {e}")
            }
            BudgetedError::Commit(e) => {
                write!(f, "reservation commit failed (estimator under-count): {e}")
            }
            BudgetedError::Call(e) => write!(f, "LLM call failed: {e}"),
        }
    }
}
impl std::error::Error for BudgetedError {}

/// Default Anthropic-style estimator (byte-length base with a safety margin).
/// Enable the `tiktoken` feature and use [`token_budgets::Tiktoken`] for real
/// token counts.
pub fn default_estimator() -> AnthropicEstimator<ByteLength> {
    AnthropicEstimator::<ByteLength>::new()
}

/// Run one LLM call under the shared session budget.
///
/// `call` performs the actual completion, e.g.
/// `|p| async move { agent.prompt(p).await.map_err(|e| anyhow::anyhow!(e)) }`.
/// Keeping it generic is what frees this crate from Rig's exact types.
/// Like [`prompt_budgeted`] but also returns `(reserved_uc, actual_uc)` so the
/// caller can compute the over-reservation ratio (the paper's headline cost
/// metric) on a real workload.
pub async fn prompt_budgeted_metered<F, Fut>(
    pool: &BudgetPool,
    estimator: &dyn TokenEstimator,
    pricing: &Pricing,
    max_output_tok: u64,
    input: &str,
    call: F,
) -> Result<(String, u64, u64), BudgetedError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = anyhow::Result<String>>,
{
    let in_tok = estimator.estimate(input);
    let reserved_uc = pricing.cost_uc(in_tok, max_output_tok);

    // (1) Pre-flight reservation; refuse now if it would breach the cap.
    let reservation = pool.reserve(reserved_uc).map_err(BudgetedError::Reserve)?;

    // (2) The actual completion. On failure, return the reservation to the pool.
    let output = match call(input.to_string()).await {
        Ok(o) => o,
        Err(e) => {
            reservation.cancel();
            return Err(BudgetedError::Call(e));
        }
    };

    // (3) Reconcile actual cost; unspent returns to the pool.
    let out_tok = estimator.estimate(&output);
    let actual_uc = pricing.cost_uc(in_tok, out_tok);
    reservation.commit(actual_uc).map_err(BudgetedError::Commit)?;
    Ok((output, reserved_uc, actual_uc))
}

pub async fn prompt_budgeted<F, Fut>(
    pool: &BudgetPool,
    estimator: &dyn TokenEstimator,
    pricing: &Pricing,
    max_output_tok: u64,
    input: &str,
    call: F,
) -> Result<String, BudgetedError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = anyhow::Result<String>>,
{
    prompt_budgeted_metered(pool, estimator, pricing, max_output_tok, input, call)
        .await
        .map(|(out, _reserved, _actual)| out)

}