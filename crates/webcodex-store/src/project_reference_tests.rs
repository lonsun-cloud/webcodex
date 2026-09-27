use super::Database;

fn root(hex: char) -> String {
    format!("wc_projroot_{}", hex.to_string().repeat(64))
}

#[test]
fn project_references_are_stable_isolated_and_restart_safe() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("project-refs.db");

    let db = Database::open(&path).unwrap();
    let first = db
        .get_or_create_project_reference("principal-a", "agent:special:webcodex", &root('1'), 1)
        .unwrap();
    assert_eq!(first.ref_index, 1);

    let replay = db
        .get_or_create_project_reference("principal-a", "agent:special:webcodex", &root('1'), 2)
        .unwrap();
    assert_eq!(replay, first);

    let second_project = db
        .get_or_create_project_reference("principal-a", "agent:oe:webcodex", &root('2'), 3)
        .unwrap();
    assert_eq!(second_project.ref_index, 2);

    let other_principal = db
        .get_or_create_project_reference("principal-b", "agent:special:webcodex", &root('1'), 4)
        .unwrap();
    assert_eq!(other_principal.ref_index, 1);

    let reincarnated = db
        .get_or_create_project_reference("principal-a", "agent:special:webcodex", &root('3'), 5)
        .unwrap();
    assert_eq!(reincarnated.ref_index, 3);
    drop(db);

    let reopened = Database::open(&path).unwrap();
    assert_eq!(
        reopened
            .lookup_project_reference("principal-a", 1)
            .unwrap()
            .unwrap(),
        first
    );
    assert_eq!(
        reopened
            .lookup_project_reference("principal-a", 2)
            .unwrap()
            .unwrap(),
        second_project
    );
    assert_eq!(
        reopened
            .lookup_project_reference("principal-a", 3)
            .unwrap()
            .unwrap(),
        reincarnated
    );
    assert_eq!(
        reopened
            .lookup_project_reference("principal-b", 1)
            .unwrap()
            .unwrap(),
        other_principal
    );
    assert!(
        reopened
            .lookup_project_reference("principal-b", 2)
            .unwrap()
            .is_none(),
        "principal namespaces must not expose another principal's mapping"
    );
}
