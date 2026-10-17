//! Settings and the exchange format (ADR 0027): version 1 carries none.

use super::*;

#[test]
fn settings_do_not_reach_the_export() {
    let (mut env, _, _) = sample_env();
    let plain = env.export();
    env.store.set_desired_retention(0.85).unwrap();
    env.store.set_max_new_per_day(3).unwrap();
    env.store.set_max_reviews_per_day(4).unwrap();
    env.store.set_day_start_hour(6).unwrap();
    let doc = json_of(&env.export());
    assert_eq!(doc["settings"], json!({}));
    assert_eq!(doc["format_version"], json!(1));
    // Only the schedule-independent data is exported, so the bytes of the
    // file do not depend on the settings.
    assert_eq!(env.export(), plain);
}

#[test]
fn an_import_leaves_the_settings_alone() {
    let (source, _, _) = sample_env();
    let mut target = Env::new();
    target.store.set_max_new_per_day(3).unwrap();
    target.import(&source.export()).unwrap();
    assert_eq!(target.store.settings().unwrap().max_new_per_day, 3);
}

#[test]
fn an_import_schedules_with_the_stored_retention() {
    let (source, _, card) = sample_env();
    let mut target = Env::new();
    target.store.set_desired_retention(0.80).unwrap();
    // On the worker's own connection, as the app imports.
    let mut worker = target.concurrent();
    let plan = worker
        .plan_import_bytes(&source.export(), &Limits::DEFAULT)
        .unwrap();
    worker.apply_import_at(&plan, T_EXPORT).unwrap();

    let events = target.store.review_events(&card).unwrap();
    let at_80 = Scheduler::new(0.80)
        .unwrap()
        .rebuild_state(&card, &events)
        .unwrap();
    assert_ne!(
        at_80,
        Scheduler::default().rebuild_state(&card, &events).unwrap(),
        "the test needs the retentions to differ"
    );
    assert_eq!(target.store.card_state(&card).unwrap(), at_80);
}
