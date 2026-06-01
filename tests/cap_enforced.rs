//! Deterministic, OFFLINE proof of the core property: under heavy delegation
//! fan-out the shared pool refuses calls once the cap is reached, and cumulative
//! spend never exceeds the cap. No API key, no network, no Rig — run with:
//!
//!   cargo test --test cap_enforced
//!
//! Calibration note: token_budgets' AnthropicEstimator is byte-length * 2.0
//! margin, so estimate(s) == 2 * s.len(). The numbers below are chosen so the
//! per-call worst-case reservation fits well inside the cap (first calls are
//! served) while the cumulative actual spend over the loop exhausts the pool
//! (later calls are refused). The mock output is sized to fit max_output_tok so
//! commit() succeeds with a refund rather than erroring.

use token_budgets::BudgetPool;
use token_budgets_rig::{default_estimator, prompt_budgeted, usd_to_uc, BudgetedError, Pricing};

#[tokio::test]
async fn pool_refuses_and_never_exceeds_cap_under_fanout() {
    let cap = usd_to_uc(0.05); // $0.05 hard ceiling == 5_000_000 micro-cents
    let pool = BudgetPool::new(cap, cap).unwrap();
    let est = default_estimator(); // byte-length * 2.0
    let price = Pricing::per_million_usd(3.0, 15.0); // $3 in / $15 out per 1e6 tok
    let max_output_tok = 200u64;

    let (mut served, mut refused) = (0usize, 0usize);
    for i in 0..1_000 {
        // input ~171 chars -> ~342 estimated input tokens
        let input = format!("subtask {i}: {}", "context ".repeat(20));
        // mock sub-agent: 70-char answer -> 140 est output tokens (< max_output_tok)
        let res = prompt_budgeted(&pool, &est, &price, max_output_tok, &input, |_p| async {
            Ok::<_, anyhow::Error>("answer ".repeat(10))
        })
        .await;

        match res {
            Ok(_) => served += 1,
            Err(BudgetedError::Reserve(_)) => refused += 1, // pre-flight cap refusal
            Err(e) => panic!("unexpected error: {e}"),
        }

        // The invariant that must hold on EVERY iteration.
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
    // A sub-agent emits far more than reserved -> commit must error, surfacing
    // the estimator under-count rather than silently overspending.
    let cap = usd_to_uc(1.0);
    let pool = BudgetPool::new(cap, cap).unwrap();
    let est = default_estimator();
    let price = Pricing::per_million_usd(3.0, 15.0);

    let res = prompt_budgeted(&pool, &est, &price, /*max_out*/ 4, "tiny prompt", |_p| async {
        Ok::<_, anyhow::Error>("x".repeat(100_000)) // vastly exceeds the 4-token reservation
    })
    .await;

    assert!(matches!(res, Err(BudgetedError::Commit(_))));
}