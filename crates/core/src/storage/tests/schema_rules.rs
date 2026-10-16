//! Schema rules.

use super::*;

#[test]
fn review_events_are_append_only() {
    let e = env();
    let store = e.open(DAY0).unwrap();
    insert_sample(&store.conn);

    for sql in [
        "UPDATE review_events SET grade = 'fail'",
        "DELETE FROM review_events",
    ] {
        let err = store.conn.execute(sql, []).unwrap_err();
        assert!(err.to_string().contains("append-only"), "{sql}: {err}");
    }
    assert_eq!(count(&store.conn, "review_events"), 1);
}

#[test]
fn card_with_reviews_cannot_be_deleted() {
    let e = env();
    let store = e.open(DAY0).unwrap();
    insert_sample(&store.conn);
    assert!(
        store
            .conn
            .execute("DELETE FROM cards WHERE id = ?1", [CARD])
            .is_err()
    );
    assert!(
        store
            .conn
            .execute("DELETE FROM decks WHERE id = ?1", [DECK])
            .is_err(),
        "deck with cards"
    );
}

#[test]
fn deleting_a_card_without_reviews_drops_its_cached_schedule() {
    let e = env();
    let store = e.open(DAY0).unwrap();
    insert_sample(&store.conn);
    store
        .conn
        .execute_batch(&format!(
            "INSERT INTO cards (id, deck_id, front_md, back_md, created_at, updated_at)
                 VALUES ('{CARD2}', '{DECK}', 'f', 'b', 1, 1);
             INSERT INTO card_schedule VALUES ('{CARD2}', 'learning', 3, 1.0, 5.0, 2, 1, 0);
             DELETE FROM cards WHERE id = '{CARD2}';"
        ))
        .unwrap();
    assert_eq!(count(&store.conn, "card_schedule"), 1);
}

#[test]
fn schema_rejects_bad_values() {
    let e = env();
    let store = e.open(DAY0).unwrap();
    insert_sample(&store.conn);
    for sql in [
        // Grade outside Fail/Pass.
        format!(
            "INSERT INTO review_events VALUES ('01920000-0000-7000-8000-000000000009', '{CARD}', 5, 'good', 0)"
        ),
        // Negative duration.
        format!(
            "INSERT INTO review_events VALUES ('01920000-0000-7000-8000-000000000009', '{CARD}', 5, 'pass', -1)"
        ),
        // Id not a lowercase 36-character UUID.
        "INSERT INTO decks (id, name, name_key, created_at, updated_at) VALUES ('short', 'x', 'x', 1, 1)".to_owned(),
        "INSERT INTO decks (id, name, name_key, created_at, updated_at) VALUES ('01920000-0000-7000-8000-00000000000A', 'x', 'x', 1, 1)".to_owned(),
        // STRICT: text in an integer column.
        "INSERT INTO decks (id, name, name_key, created_at, updated_at) VALUES ('01920000-0000-7000-8000-00000000000b', 'x', 'x', 'now', 1)"
            .to_owned(),
        // Unknown card.
        "INSERT INTO review_events VALUES ('01920000-0000-7000-8000-000000000009', '01920000-0000-7000-8000-0000000000ff', 5, 'pass', 0)".to_owned(),
    ] {
        assert!(store.conn.execute(&sql, []).is_err(), "{sql}");
    }
}
