//! Extra Rig workload (breadth): a GROWING-CONTEXT multi-turn conversation.
//! Unlike delegation_demo's uniform tasks, each turn re-sends the accumulated
//! transcript, so per-call cost RISES turn over turn. This stresses the cap
//! under a non-uniform cost shape: reservations grow until one is refused.
//!
//!   ANTHROPIC_API_KEY=sk-ant-... cargo run --example workload_multiturn

use std::time::Instant;
use token_budgets::BudgetPool;
use token_budgets_rig::{default_estimator, prompt_budgeted_metered, usd_to_uc, BudgetedError, Pricing};

use rig_core::client::{CompletionClient, ProviderClient};
use rig_core::completion::Prompt;
use rig_core::providers::anthropic;

const MODEL: &str = "claude-haiku-4-5";
const SESSION_CAP_USD: f64 = 0.05;
const N_TURNS: usize = 30;
const MAX_OUTPUT_TOK: u64 = 2048;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let pricing = Pricing::per_million_usd(1.0, 5.0);
    let estimator = default_estimator();

    let client = anthropic::Client::from_env()?;
    let agent = client
        .agent(MODEL)
        .preamble("You are a terse assistant. Answer each question in one sentence.")
        .max_tokens(100)
        .build();

    let cap = usd_to_uc(SESSION_CAP_USD);
    let pool = BudgetPool::new(cap, cap).expect("valid pool");
    let (mut served, mut refused) = (0usize, 0usize);
    let mut transcript = String::new();
    let start = Instant::now();

    for turn in 0..N_TURNS {
        transcript.push_str(&format!("\nUser: Question {turn}: name one risk of unbounded agent retries.\n"));
        let prompt = transcript.clone(); // grows every turn -> per-call cost rises
        let in_est = estimator_chars(&prompt);
        match prompt_budgeted_metered(&pool, &estimator, &pricing, MAX_OUTPUT_TOK, &prompt, |p| async {
            agent.prompt(p).await.map_err(|e| anyhow::anyhow!(e))
        })
            .await
        {
            Ok((out, reserved, actual)) => {
                served += 1;
                transcript.push_str(&format!("Assistant: {out}\n"));
                println!("turn {turn:2}: ctx≈{in_est}b served  reserved=${:.5} actual=${:.5}",
                         reserved as f64 / 1e8, actual as f64 / 1e8);
            }
            Err(BudgetedError::Reserve(_)) => {
                refused += 1;
                println!("turn {turn:2}: ctx≈{in_est}b REFUSED (cap reached as context grew)");
                break; // once the growing context can't be afforded, the rest won't be either
            }
            Err(e) => return Err(anyhow::anyhow!(e.to_string())),
        }
        assert!(pool.invariant_holds());
    }

    let spent_uc = cap - pool.available();
    println!("--- growing-context workload ---");
    println!("served={served} refused={refused} elapsed={:?}", start.elapsed());
    println!("spent=${:.5} cap=${:.2}  CAP RESPECTED: {}",
             spent_uc as f64 / 1e8, SESSION_CAP_USD, spent_uc <= cap);
    Ok(())
}

fn estimator_chars(s: &str) -> usize { s.len() }