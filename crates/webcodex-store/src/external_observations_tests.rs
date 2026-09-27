use super::{
    Database, ExternalObservation, ExternalObservationError, MAX_EXTERNAL_OBSERVATIONS_PER_SESSION,
};

fn input(n: usize) -> ExternalObservation {
    ExternalObservation {
        adapter_id: "a".repeat(64),
        event_id: format!("{n:064x}"),
        tool: "Bash".into(),
        exit_code: None,
        recorded_at: 1_000,
    }
}

#[test]
fn external_observations_reopen_replay_conflict_and_scope() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = Database::open(&path).unwrap();
    assert!(
        db.record_external_observation("wc_sess_test", "agent:r:p", input(1))
            .unwrap()
            .1
    );
    drop(db);
    let db = Database::open(&path).unwrap();
    let mut later = input(1);
    later.recorded_at += 100;
    let (stored, inserted) = db
        .record_external_observation("wc_sess_test", "agent:r:p", later)
        .unwrap();
    assert!(!inserted);
    assert_eq!(stored, input(1));
    let mut conflict = input(1);
    conflict.exit_code = Some(0);
    assert_eq!(
        db.record_external_observation("wc_sess_test", "agent:r:p", conflict),
        Err(ExternalObservationError::Conflict)
    );
    assert_eq!(
        db.record_external_observation("wc_sess_test", "agent:r:other", input(1)),
        Err(ExternalObservationError::Conflict)
    );
    assert!(db
        .list_external_observations("wc_sess_test", "agent:r:other")
        .unwrap()
        .is_empty());
    assert!(
        db.record_external_observation("wc_sess_other", "agent:r:p", input(1))
            .unwrap()
            .1
    );
    assert_eq!(
        db.list_external_observations("wc_sess_test", "agent:r:p")
            .unwrap(),
        vec![input(1)]
    );
}

#[test]
fn external_observations_bound_history_without_forgetting_replay_identity() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("state.db")).unwrap();
    for n in 0..MAX_EXTERNAL_OBSERVATIONS_PER_SESSION {
        db.record_external_observation("wc_sess_test", "agent:r:p", input(n))
            .unwrap();
    }
    assert_eq!(
        db.record_external_observation("wc_sess_test", "agent:r:p", input(300)),
        Err(ExternalObservationError::Capacity)
    );
    assert!(
        !db.record_external_observation("wc_sess_test", "agent:r:p", input(0))
            .unwrap()
            .1
    );
    assert_eq!(
        db.list_external_observations("wc_sess_test", "agent:r:p")
            .unwrap()
            .len(),
        MAX_EXTERNAL_OBSERVATIONS_PER_SESSION
    );
}

#[test]
fn external_observations_concurrent_replay_is_one_record() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let a = Database::open(&path).unwrap();
    let b = Database::open(&path).unwrap();
    let handles: Vec<_> = [a, b]
        .into_iter()
        .map(|db| {
            std::thread::spawn(move || {
                db.record_external_observation("wc_sess_test", "agent:r:p", input(1))
                    .unwrap()
                    .1
            })
        })
        .collect();
    assert_eq!(
        handles
            .into_iter()
            .filter_map(|h| h.join().unwrap().then_some(()))
            .count(),
        1
    );
}

#[test]
fn external_observations_reject_raw_text_and_rollback_failed_insert() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("state.db")).unwrap();
    let mut bad = input(0);
    bad.tool = "Bash echo secret".into();
    assert_eq!(
        db.record_external_observation("wc_sess_test", "agent:r:p", bad),
        Err(ExternalObservationError::InvalidInput)
    );
    db.conn_for_tests().execute_batch("CREATE TRIGGER fail_observation BEFORE INSERT ON wc_external_observations BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert_eq!(
        db.record_external_observation("wc_sess_test", "agent:r:p", input(1)),
        Err(ExternalObservationError::Storage)
    );
    assert!(db
        .list_external_observations("wc_sess_test", "agent:r:p")
        .unwrap()
        .is_empty());
    db.conn_for_tests()
        .execute_batch("DROP TRIGGER fail_observation")
        .unwrap();
    assert!(
        db.record_external_observation("wc_sess_test", "agent:r:p", input(1))
            .unwrap()
            .1
    );
}
