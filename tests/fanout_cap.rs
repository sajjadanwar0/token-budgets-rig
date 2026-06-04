use token_budgets::BudgetPool;
use token_budgets_rig::{default_estimator, prompt_budgeted_metered, usd_to_uc, BudgetedError, Pricing};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_subagents_cannot_exceed_shared_cap() {
    let cap = usd_to_uc(0.05);
    let pool = BudgetPool::new(cap, cap).unwrap();
    let pricing = Pricing::per_million_usd(1.0, 5.0);

    let mut handles = Vec::new();
    for sub in 0..8u32 {
        let pool = pool.clone(); 
        handles.push(tokio::spawn(async move {
            let est = default_estimator();
            let mut served = 0u64;
            for t in 0..1_000u32 {
                let input = format!("[sub {sub}] task {t}: {}", "context ".repeat(20));
                match prompt_budgeted_metered(&pool, &est, &pricing, 2048, &input, |_p| async {
                    Ok::<_, anyhow::Error>("answer ".repeat(10))
                })
                    .await
                {
                    Ok(_) => served += 1,
                    Err(BudgetedError::Reserve(_)) => {}
                    Err(e) => panic!("unexpected: {e}"),
                }

                assert!(pool.invariant_holds());
                assert!(pool.available() <= cap);
            }
            served
        }));
    }

    let mut total_served = 0u64;
    for h in handles {
        total_served += h.await.unwrap();
    }

    let spent = cap - pool.available();
    
    assert!(spent <= cap, "cumulative spend across sub-agents exceeded the cap");
    assert!(total_served > 0);
    assert!(pool.invariant_holds());
    println!("subagents=8 attempts=8000 total_served={total_served} global_spend_uc={spent} cap_uc={cap}");
}