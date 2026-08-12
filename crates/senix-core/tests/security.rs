use std::{
    collections::BTreeSet,
    sync::Arc,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use senix_core::{
    AccessPolicy, Error, ManagementAction, ResourceRef, SecurityController, SqliteStateStore,
};

#[test]
fn owner_bootstrap_is_one_time_and_credentials_never_list_raw_keys() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStateStore::open(dir.path().join("state.db")).unwrap());
    let security = SecurityController::new(store);

    let issued = security.bootstrap_owner_key("local owner").unwrap();
    let owner = security.authenticate(&issued.api_key).unwrap();
    assert!(matches!(
        security.bootstrap_owner_key("second owner"),
        Err(Error::CredentialAlreadyInitialized)
    ));

    let listed = security.list_credentials(&owner).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].credential_id, issued.credential_id);
    assert!(
        !serde_json::to_string(&listed)
            .unwrap()
            .contains(&issued.api_key)
    );
}

#[test]
fn api_key_enforces_action_instance_and_expiry() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStateStore::open(dir.path().join("state.db")).unwrap());
    let security = SecurityController::new(store);
    let owner_key = security.bootstrap_owner_key("owner").unwrap();
    let owner = security.authenticate(&owner_key.api_key).unwrap();
    let expires_at_ms = now_ms() + 80;
    let issued = security
        .issue_key(
            &owner,
            "deployment",
            AccessPolicy {
                all_resources: false,
                actions: BTreeSet::from([
                    ManagementAction::InstanceRead,
                    ManagementAction::InstanceDrain,
                ]),
                instance_ids: BTreeSet::from(["instance-a".to_owned()]),
            },
            Some(expires_at_ms),
        )
        .unwrap();
    let deployment = security.authenticate(&issued.api_key).unwrap();

    security
        .authorize(
            &deployment,
            ManagementAction::InstanceDrain,
            &ResourceRef::Instance("instance-a".to_owned()),
        )
        .unwrap();
    assert!(matches!(
        security.authorize(
            &deployment,
            ManagementAction::InstanceRejoin,
            &ResourceRef::Instance("instance-a".to_owned()),
        ),
        Err(Error::Forbidden { .. })
    ));
    assert!(matches!(
        security.authorize(
            &deployment,
            ManagementAction::InstanceRead,
            &ResourceRef::Instance("instance-b".to_owned()),
        ),
        Err(Error::Forbidden { .. })
    ));

    thread::sleep(Duration::from_millis(100));
    assert!(matches!(
        security.authenticate(&issued.api_key),
        Err(Error::CredentialExpired)
    ));
}

#[test]
fn owner_account_revokes_bootstrap_key_and_sessions_are_statelessly_invalidated() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStateStore::open(dir.path().join("state.db")).unwrap());
    let security = SecurityController::new(store);
    let bootstrap = security.bootstrap_owner_key("bootstrap owner").unwrap();

    security
        .bootstrap_owner_account("admin", "correct horse battery staple")
        .unwrap();
    assert!(matches!(
        security.authenticate(&bootstrap.api_key),
        Err(Error::CredentialRevoked)
    ));

    let issued = security
        .login_owner("admin", "correct horse battery staple", 60_000)
        .unwrap();
    let principal = security.authenticate_owner_session(&issued.token).unwrap();
    security.logout_owner(&principal).unwrap();
    assert!(matches!(
        security.authenticate_owner_session(&issued.token),
        Err(Error::InvalidOwnerSession)
    ));

    security
        .reset_owner_password("new correct horse battery staple")
        .unwrap();
    assert!(matches!(
        security.login_owner("admin", "correct horse battery staple", 60_000),
        Err(Error::InvalidOwnerLogin)
    ));
    security
        .login_owner("admin", "new correct horse battery staple", 60_000)
        .unwrap();
}

#[test]
fn api_keys_may_apply_an_approved_change_but_can_never_approve_one() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStateStore::open(dir.path().join("state.db")).unwrap());
    let security = SecurityController::new(store);
    let owner_key = security.bootstrap_owner_key("owner").unwrap();
    let owner = security.authenticate(&owner_key.api_key).unwrap();

    let approval_policy = AccessPolicy {
        all_resources: true,
        actions: BTreeSet::from([ManagementAction::ChangeApprove]),
        instance_ids: BTreeSet::new(),
    };
    assert!(matches!(
        security.issue_key(&owner, "self-approver", approval_policy, None),
        Err(Error::InvalidState(_))
    ));

    let apply_key = security
        .issue_key(
            &owner,
            "change-runner",
            AccessPolicy {
                all_resources: true,
                actions: BTreeSet::from([
                    ManagementAction::ChangeRead,
                    ManagementAction::ChangeApply,
                ]),
                instance_ids: BTreeSet::new(),
            },
            None,
        )
        .unwrap();
    let runner = security.authenticate(&apply_key.api_key).unwrap();
    security
        .authorize(&runner, ManagementAction::ChangeApply, &ResourceRef::Global)
        .unwrap();
    assert!(matches!(
        security.authorize(
            &runner,
            ManagementAction::ChangeApprove,
            &ResourceRef::Global,
        ),
        Err(Error::Forbidden { .. })
    ));
}

fn now_ms() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}
