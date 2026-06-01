//! Live N=1 demo: a coordinator delegates subtasks to a Rig agent under a hard
//! session dollar cap. Requires an API key:
//!
//!   ANTHROPIC_API_KEY=sk-ant-... cargo run --example delegation_demo
//!
//! Every delegation goes through `prompt_budgeted_metered` against one shared
//! `BudgetPool`. Once cumulative spend reaches the cap, further delegations are
//! refused PRE-FLIGHT (BudgetedError::Reserve) and the session never overspends.
//! The run also reports the over-reservation ratio (reserved / actual) -- the
//! paper's headline cost metric -- measured on a real framework.
//!
//! RIG VERSION SURFACE: the lines tagged `// RIG:` are the only Rig-specific
//! code. Pinned to rig-core 0.37 (imported as `rig_core`; the 0.36 `rig` alias
//! was dropped). Confirm with `cargo doc -p rig-core --open`.

use std::time::Instant;
use token_budgets::BudgetPool;
use token_budgets_rig::{default_estimator, prompt_budgeted_metered, usd_to_uc, BudgetedError, Pricing};

// RIG: imports. ProviderClient provides `from_env`; CompletionClient provides `.agent()`.
use rig_core::client::{CompletionClient, ProviderClient};
use rig_core::completion::Prompt;
use rig_core::providers::anthropic;

const MODEL: &str = "claude-haiku-4-5"; // RIG: a model id your account can call
const SESSION_CAP_USD: f64 = 0.05; // small enough that the cap binds within N_SUBTASKS
const N_SUBTASKS: usize = 40;
const MAX_OUTPUT_TOK: u64 = 2048; // worst-case output (estimator bytes*2 units) for a <=100-token answer

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Claude Haiku 4.5 rate card, verified May 2026: $1 / $5 per 1M tok.
    let pricing = Pricing::per_million_usd(1.0, 5.0);
    let estimator = default_estimator();

    // RIG: build the client + agent.
    let client = anthropic::Client::from_env()?; // 0.37: returns Result
    let agent = client
        .agent(MODEL)
        .preamble("You are a terse worker. Answer in one sentence.")
        .max_tokens(100) // RIG: hard-bound output so the worst-case reservation is sound
        .build();

    let subtasks: Vec<String> = (0..N_SUBTASKS)
        .map(|i| format!("Summarise the risk of unbounded retries in agent step {i}."))
        .collect();

    let cap = usd_to_uc(SESSION_CAP_USD);
    let pool = BudgetPool::new(cap, cap).expect("valid pool");
    let (mut served, mut refused, mut under_estimates) = (0usize, 0usize, 0usize);
    let (mut total_reserved, mut total_actual) = (0u64, 0u64);
    let start = Instant::now();

    for task in &subtasks {
        let res = prompt_budgeted_metered(&pool, &estimator, &pricing, MAX_OUTPUT_TOK, task, |p| async {
            agent.prompt(p).await.map_err(|e| anyhow::anyhow!(e)) // RIG: the completion call
        })
            .await;

        match res {
            Ok((_out, reserved, actual)) => {
                served += 1;
                total_reserved += reserved;
                total_actual += actual;
            }
            Err(BudgetedError::Reserve(_)) => refused += 1, // cap reached: refuse pre-flight
            Err(BudgetedError::Commit(_)) => under_estimates += 1, // output exceeded reservation; bound max_tokens lower
            Err(e) => return Err(anyhow::anyhow!(e.to_string())),
        }
        assert!(pool.invariant_holds(), "cap invariant violated");
    }

    let spent_uc = cap - pool.available();
    let over_res = if total_actual > 0 {
        total_reserved as f64 / total_actual as f64
    } else {
        f64::NAN
    };
    // What the full N-task workload would have cost unguarded (extrapolated from
    // the served calls' mean actual cost) -- the overrun the cap prevented.
    let projected_unguarded_uc = if served > 0 {
        (total_actual as f64 / served as f64) * N_SUBTASKS as f64
    } else {
        0.0
    };

    println!("--- N=1 deployment: token-budgets on Rig (rig-core 0.37) ---");
    println!("model={MODEL}  cap=${:.4}  subtasks={N_SUBTASKS}", SESSION_CAP_USD);
    println!("served={served}  refused={refused}  under_estimates={under_estimates}  elapsed={:?}", start.elapsed());
    println!("actual spend       = ${:.5}", spent_uc as f64 / 1e8);
    println!("over-reservation   = {:.2}x  (reserved ${:.5} / actual ${:.5})",
             over_res, total_reserved as f64 / 1e8, total_actual as f64 / 1e8);
    println!("projected unguarded= ${:.5}  for all {N_SUBTASKS} tasks (would breach cap: {})",
             projected_unguarded_uc / 1e8, projected_unguarded_uc as u64 > cap);
    println!("CAP RESPECTED: {}  (spent_uc={spent_uc} <= cap_uc={cap})", spent_uc <= cap);
    Ok(())
}