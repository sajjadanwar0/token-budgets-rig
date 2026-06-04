use std::sync::Arc;
use std::time::Instant;
use token_budgets::BudgetPool;
use token_budgets_rig::{default_estimator, prompt_budgeted_metered, usd_to_uc, BudgetedError, Pricing};
use rig_core::client::{CompletionClient, ProviderClient};
use rig_core::completion::Prompt;
use rig_core::providers::anthropic;

const MODEL: &str = "claude-haiku-4-5";
const SESSION_CAP_USD: f64 = 0.05;
const N_SUBAGENTS: usize = 4;
const TASKS_PER_SUBAGENT: usize = 15;
const MAX_OUTPUT_TOK: u64 = 2048;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> anyhow::Result<()> {
    let pricing = Pricing::per_million_usd(1.0, 5.0); 

    let client = anthropic::Client::from_env()?;
    let agent = Arc::new(
        client
            .agent(MODEL)
            .preamble("You are a terse worker. Answer in one sentence.")
            .max_tokens(100)
            .build(),
    );

    let cap = usd_to_uc(SESSION_CAP_USD);
    let pool = BudgetPool::new(cap, cap).expect("valid pool");
    let start = Instant::now();

    let mut handles = Vec::new();
    
    for sub in 0..N_SUBAGENTS {
        let pool = pool.clone(); 
        let agent = Arc::clone(&agent);
        handles.push(tokio::spawn(async move {
            let est = default_estimator();
            let (mut served, mut refused, mut spent) = (0usize, 0usize, 0u64);
            
            for t in 0..TASKS_PER_SUBAGENT {
                let task = format!("[sub {sub}] step {t}: summarise one retry-storm risk.");
                match prompt_budgeted_metered(&pool, &est, &pricing, MAX_OUTPUT_TOK, &task, |p| async {
                    agent.prompt(p).await.map_err(|e| anyhow::anyhow!(e)) // RIG: completion call
                })
                    .await
                {
                    Ok((_, _, actual)) => {
                        served += 1;
                        spent += actual;
                    }
                    Err(BudgetedError::Reserve(_)) => refused += 1, // global cap reached
                    Err(_) => {} // under-estimate tolerated
                }
            }
            (sub, served, refused, spent)
        }));
    }

    let (mut tot_served, mut tot_refused) = (0usize, 0usize);
    
    for h in handles {
        let (sub, served, refused, spent) = h.await?;
        tot_served += served;
        tot_refused += refused;
        println!("  sub-agent {sub}: served={served} refused={refused} spent=${:.5}", spent as f64 / 1e8);
    }

    let spent_uc = cap - pool.available();
    println!("--- multi-agent fan-out: {N_SUBAGENTS} concurrent sub-agents, ONE BudgetPool ---");
    println!("served={tot_served} refused={tot_refused} elapsed={:?}", start.elapsed());
    println!("GLOBAL spend = ${:.5}  cap = ${:.2}", spent_uc as f64 / 1e8, SESSION_CAP_USD);
    println!("pool invariant holds: {}", pool.invariant_holds());
    println!("CAP RESPECTED ACROSS ALL SUB-AGENTS: {}", spent_uc <= cap);
    Ok(())
}