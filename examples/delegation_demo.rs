use std::time::Instant;
use token_budgets::BudgetPool;
use token_budgets_rig::{default_estimator, prompt_budgeted_metered, usd_to_uc, BudgetedError, Pricing};
use rig_core::client::{CompletionClient, ProviderClient};
use rig_core::completion::Prompt;
use rig_core::providers::anthropic;

const MODEL: &str = "claude-haiku-4-5";
const SESSION_CAP_USD: f64 = 0.05; 
const N_SUBTASKS: usize = 40;
const MAX_OUTPUT_TOK: u64 = 2048;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let pricing = Pricing::per_million_usd(1.0, 5.0);
    let estimator = default_estimator();

    let client = anthropic::Client::from_env()?; 
    let agent = client
        .agent(MODEL)
        .preamble("You are a terse worker. Answer in one sentence.")
        .max_tokens(100) 
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
            agent.prompt(p).await.map_err(|e| anyhow::anyhow!(e)) 
        })
            .await;

        match res {
            Ok((_out, reserved, actual)) => {
                served += 1;
                total_reserved += reserved;
                total_actual += actual;
            }
            Err(BudgetedError::Reserve(_)) => refused += 1,
            Err(BudgetedError::Commit(_)) => under_estimates += 1, 
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