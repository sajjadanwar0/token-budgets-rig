use std::future::Future;
use token_budgets::{BudgetError, BudgetPool, AnthropicEstimator, ByteLength, TokenEstimator};

#[derive(Clone, Copy, Debug)]
pub struct Pricing {
    pub input_uc_per_tok: u64,
    pub output_uc_per_tok: u64,
}

impl Pricing {
    pub fn per_million_usd(input_usd: f64, output_usd: f64) -> Self {
        Self {
            input_uc_per_tok: (input_usd * 100.0).round() as u64,
            output_uc_per_tok: (output_usd * 100.0).round() as u64,
        }
    }

    #[inline]
    pub fn cost_uc(&self, in_tok: u64, out_tok: u64) -> u64 {
        in_tok
            .saturating_mul(self.input_uc_per_tok)
            .saturating_add(out_tok.saturating_mul(self.output_uc_per_tok))
    }
}

pub fn usd_to_uc(usd: f64) -> u64 {
    (usd * 100.0 * 1_000_000.0).round() as u64
}

#[derive(Debug)]
pub enum BudgetedError {
    Reserve(BudgetError),
    Commit(BudgetError),
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

pub fn default_estimator() -> AnthropicEstimator<ByteLength> {
    AnthropicEstimator::<ByteLength>::new()
}

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
    let reservation = pool.reserve(reserved_uc).map_err(BudgetedError::Reserve)?;

    let output = match call(input.to_string()).await {
        Ok(o) => o,
        Err(e) => {
            reservation.cancel();
            return Err(BudgetedError::Call(e));
        }
    };

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