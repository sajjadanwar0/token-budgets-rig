use token_budgets::BudgetPool;

fn main() {
    let pool = BudgetPool::new(1_000, 1_000).unwrap();
    let r = pool.reserve(100).unwrap();
    let _dup = r.clone(); 
    let _ = r;
}
