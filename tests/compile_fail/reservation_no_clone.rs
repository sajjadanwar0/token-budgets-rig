// Non-bypassability (compile-time): a Reservation cannot be cloned, so a
// sub-agent cannot duplicate its budget slice to spend it more than once.
use token_budgets::BudgetPool;

fn main() {
    let pool = BudgetPool::new(1_000, 1_000).unwrap();
    let r = pool.reserve(100).unwrap();
    let _dup = r.clone(); // Reservation: !Clone  -> compile error
    let _ = r;
}
