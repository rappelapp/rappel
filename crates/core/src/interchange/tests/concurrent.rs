//! A second connection.

use super::*;

impl Env {
    /// A second store on the same database, as a worker thread opens it.
    fn concurrent(&self) -> Store {
        Store::open_concurrent(&self.tmp.path().join("data")).unwrap()
    }
}

#[test]
fn an_export_on_another_thread_sees_only_committed_data() {
    let (env, deck, _) = sample_env();
    let committed = env.export();
    // An unfinished write on the main connection...
    let tx = env.store.conn.unchecked_transaction().unwrap();
    tx.execute("UPDATE decks SET name = 'Changed' WHERE id = ?1", [&deck])
        .unwrap();
    // ...is not in an export made meanwhile on another thread.
    let worker = env.concurrent();
    let bytes = std::thread::spawn(move || export(&worker)).join().unwrap();
    assert_eq!(bytes, committed);
    drop(tx); // rolls back
    assert_eq!(env.export(), committed);
}

#[test]
fn an_import_on_another_thread_while_the_main_store_reads() {
    let (source, _, _) = sample_env();
    let bytes = source.export();
    let mut target = Env::new();
    let mut worker = target.concurrent();
    // A read transaction on the main connection (its first read takes the
    // snapshot) does not block the write, and keeps seeing its snapshot.
    let reading = target.store.conn.unchecked_transaction().unwrap();
    let cards_in = |tx: &rusqlite::Transaction<'_>| -> i64 {
        tx.query_row("SELECT count(*) FROM cards", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(cards_in(&reading), 0);
    let plan = worker.plan_import_bytes(&bytes, &Limits::DEFAULT).unwrap();
    let report = std::thread::spawn(move || worker.apply_import_at(&plan, T_EXPORT))
        .join()
        .unwrap()
        .unwrap();
    assert!(report.cards > 0);
    assert_eq!(cards_in(&reading), 0);
    reading.finish().unwrap();
    assert_eq!(target.export(), bytes);

    // Ids issued by the main store afterwards stay above the imported ones.
    let largest: String = target
        .store
        .conn
        .query_row("SELECT max(id) FROM decks", [], |r| r.get(0))
        .unwrap();
    let deck = target.store.create_deck("After the import").unwrap();
    assert!(deck.id > largest);
}

#[test]
fn a_change_on_the_main_store_makes_a_worker_plan_stale() {
    let (source, target, _, card) = diverged();
    let mut worker = target.concurrent();
    let plan = worker
        .plan_import_bytes(&source.export(), &Limits::DEFAULT)
        .unwrap();
    assert_eq!(plan.summary().conflicts, []);
    let mut target = target;
    target
        .store
        .edit_card(&card, "changed", "meanwhile")
        .unwrap();
    let before = target.export();
    assert!(matches!(
        worker.apply_import_at(&plan, T_EXPORT),
        Err(Error::ImportPlanStale)
    ));
    assert_eq!(target.export(), before);
}

#[test]
fn what_a_worker_returns_can_cross_threads() {
    fn assert_send<T: Send + 'static>() {}
    assert_send::<ImportPlan>();
    assert_send::<ImportReport>();
    assert_send::<ExportSummary>();
    assert_send::<crate::Error>();
}
