//! Test support for D-028 (energy efficiency). A test counts the SQLite
//! virtual machine steps of an operation at two data sizes. The count does
//! not depend on the speed of the machine, so the check is deterministic.

use rusqlite::Connection;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// The SQLite VM steps that `action` runs on `db`.
pub(crate) fn vm_steps<T>(db: &Connection, action: impl FnOnce() -> T) -> (T, u64) {
    let steps = Arc::new(AtomicU64::new(0));
    let counter = Arc::clone(&steps);
    db.progress_handler(
        1,
        Some(move || {
            counter.fetch_add(1, Ordering::Relaxed);
            false
        }),
    );
    let value = action();
    db.progress_handler(1, None::<fn() -> bool>);
    (value, steps.load(Ordering::Relaxed))
}

/// D-028 rule 3: the cost at the large size stays within a small factor of
/// the cost at the small size. B-tree depth adds a little; a scan of all
/// rows multiplies the cost by the size ratio.
pub(crate) fn assert_flat_cost(name: &str, small: u64, large: u64) {
    assert!(small > 0, "{name}: no VM steps measured");
    assert!(
        large <= small * 3 / 2 + 50,
        "{name}: cost grows with data size: {small} VM steps -> {large} VM steps"
    );
}

#[test]
fn the_meter_detects_a_full_scan() {
    let steps = |rows: i64| {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t(x INTEGER)").unwrap();
        db.execute(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<?1)
             INSERT INTO t SELECT i FROM n",
            [rows],
        )
        .unwrap();
        vm_steps(&db, || {
            db.query_row("SELECT count(*) FROM t WHERE x>0", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
        })
        .1
    };
    let (small, large) = (steps(100), steps(10_000));
    assert!(large > small * 10, "{small} -> {large}");
    let result = std::panic::catch_unwind(|| assert_flat_cost("scan", small, large));
    assert!(result.is_err());
}
