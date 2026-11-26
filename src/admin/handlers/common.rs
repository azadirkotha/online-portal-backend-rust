/// True when MySQL refused the statement because of a foreign key (e.g. deleting a parent that still has children).
pub fn is_fk_violation(e: &sqlx::Error) -> bool {
    e.as_database_error().map(|d| d.is_foreign_key_violation()).unwrap_or(false)
}

pub fn parse_sort_order(raw: &str) -> i32 {
    raw.trim().parse::<i32>().unwrap_or(0)
}
