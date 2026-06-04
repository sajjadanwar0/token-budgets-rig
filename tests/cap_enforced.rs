use token_budgets::BudgetPool;
use token_budgets_rig::{default_estimator, prompt_budgeted, usd_to_uc, BudgetedError, Pricing};

#[tokio::test]
async fn pool_refuses_and_never_exceeds_cap_under_fanout() {
    let cap = usd_to_uc(0.05);
    let pool = BudgetPool::new(cap, cap).unwrap();
    let est = default_estimator();
    let price = Pricing::per_million_usd(3.0, 15.0);
    let max_output_tok = 200u64;

    let (mut served, mut refused) = (0usize, 0usize);
    for i in 0..1_000 {
        let input = format!("subtask {i}: {}", "context ".repeat(20));
        let res = prompt_budgeted(&pool, &est, &price, max_output_tok, &input, |_p| async {
            Ok::<_, anyhow::Error>("answer ".repeat(10))
        })
        .await;

        match res {
            Ok(_) => served += 1,
            Err(BudgetedError::Reserve(_)) => refused += 1,
            Err(e) => panic!("unexpected error: {e}"),
        }

        assert!(pool.invariant_holds(), "pool invariant violated");
        assert!(cap - pool.available() <= cap, "cumulative spend exceeded cap");
    }

    assert!(served > 0, "some calls should be served (got {served})");
    assert!(refused > 0, "the cap must refuse calls under fan-out (got {refused})");
    assert!(pool.available() <= cap);
    println!(
        "served={served} refused={refused} final_spend=${:.4}",
        (cap - pool.available()) as f64 / 1e8
    );
}

#[tokio::test]
async fn under_estimate_is_caught_at_commit() {
    let cap = usd_to_uc(1.0);
    let pool = BudgetPool::new(cap, cap).unwrap();
    let est = default_estimator();
    let price = Pricing::per_million_usd(3.0, 15.0);

    let res = prompt_budgeted(&pool, &est, &price, 4, "tiny prompt", |_p| async {
        Ok::<_, anyhow::Error>("x".repeat(100_000))
    })
    .await;

    assert!(matches!(res, Err(BudgetedError::Commit(_))));
}