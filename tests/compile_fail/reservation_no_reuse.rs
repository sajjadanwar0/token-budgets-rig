// Non-bypassability (compile-time): committing a Reservation consumes it, so a
// sub-agent cannot spend the same slice twice.
use token_budgets::BudgetPool;

fn main() {
    let pool = BudgetPool::new(1_000, 1_000).unwrap();
    let r = pool.reserve(100).unwrap();
    r.commit(50).unwrap(); // moves r
    r.commit(50).unwrap(); // use of moved value -> E0382
}
