use std::sync::Arc;
use std::time::Instant;
use token_budgets::BudgetPool;
use token_budgets_rig::{default_estimator, prompt_budgeted_metered, usd_to_uc, BudgetedError, Pricing};

use autoagents::core::agent::memory::SlidingWindowMemory;
use autoagents::core::agent::prebuilt::executor::{ReActAgent, ReActAgentOutput};
use autoagents::core::agent::task::Task;
use autoagents::core::agent::{AgentBuilder, AgentOutputT, DirectAgent};
use autoagents::llm::backends::anthropic::Anthropic;
use autoagents::llm::builder::LLMBuilder;
use autoagents::llm::LLMProvider;
use autoagents_derive::{agent, AgentHooks, AgentOutput};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, AgentOutput)]
pub struct WorkerOutput {
    #[output(description = "the one-sentence answer")]
    answer: String,
}

impl From<ReActAgentOutput> for WorkerOutput {
    fn from(o: ReActAgentOutput) -> Self {
        WorkerOutput { answer: o.response }
    }
}

#[agent(
    name = "worker",
    description = "A terse worker. Answer in one sentence.",
    output = WorkerOutput
)]
#[derive(Default, Clone, AgentHooks)]
pub struct Worker {}

const MODEL: &str = "claude-haiku-4-5";
const SESSION_CAP_USD: f64 = 0.05;
const N_SUBAGENTS: usize = 4;
const TASKS_PER_SUBAGENT: usize = 15;
const MAX_OUTPUT_TOK: u64 = 2048;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> anyhow::Result<()> {
    let pricing = Pricing::per_million_usd(1.0, 5.0);

    let api_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
    let llm: Arc<dyn LLMProvider> = LLMBuilder::<Anthropic>::new()
        .api_key(api_key)
        .model(MODEL)
        .max_tokens(100)
        .temperature(0.2)
        .build()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let cap = usd_to_uc(SESSION_CAP_USD);
    let pool = BudgetPool::new(cap, cap).expect("valid pool");
    let start = Instant::now();

    let mut handles = Vec::new();
    for sub in 0..N_SUBAGENTS {
        let pool = pool.clone();
        let llm = Arc::clone(&llm);
        handles.push(tokio::spawn(async move {
            let agent = AgentBuilder::<_, DirectAgent>::new(ReActAgent::new(Worker {}))
                .llm(llm)
                .memory(Box::new(SlidingWindowMemory::new(4)))
                .build()
                .await
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;

            let est = default_estimator();
            let (mut served, mut refused, mut spent) = (0usize, 0usize, 0u64);
            for t in 0..TASKS_PER_SUBAGENT {
                let task = format!("[sub {sub}] step {t}: name one risk of unbounded agent retries.");
                
                let res = prompt_budgeted_metered(&pool, &est, &pricing, MAX_OUTPUT_TOK, &task, |p| async {

                    let out: WorkerOutput = agent
                        .agent
                        .run(Task::new(p))
                        .await
                        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                    Ok(out.answer)
                })
                    .await;
                match res {
                    Ok((_, _, actual)) => { served += 1; spent += actual; }
                    Err(BudgetedError::Reserve(_)) => refused += 1,
                    Err(_) => {}
                }
            }
            
            Ok::<_, anyhow::Error>((sub, served, refused, spent))
        }));
    }

    let (mut tot_served, mut tot_refused) = (0usize, 0usize);
    for h in handles {
        let (sub, served, refused, spent) = h.await??;
        tot_served += served;
        tot_refused += refused;
        println!("  agent {sub}: served={served} refused={refused} spent=${:.5}", spent as f64 / 1e8);
    }

    
    let spent_uc = cap - pool.available();
    
    println!("--- AutoAgents fan-out: {N_SUBAGENTS} concurrent agents, ONE BudgetPool ---");
    println!("served={tot_served} refused={tot_refused} elapsed={:?}", start.elapsed());
    println!("GLOBAL spend = ${:.5}  cap = ${:.2}", spent_uc as f64 / 1e8, SESSION_CAP_USD);
    println!("pool invariant holds: {}", pool.invariant_holds());
    println!("CAP RESPECTED ACROSS ALL AGENTS: {}", spent_uc <= cap);
    
    Ok(())
}