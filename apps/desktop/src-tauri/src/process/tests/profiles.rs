//! Real owned-process tests; these are not evidence of two real ChatGPT accounts.
use crate::activity::ActivityLog;
use crate::connection_id::TunnelProfileId;
use crate::process::{ProcessKey, ProcessPhase, ProcessSupervisor};
use std::process::Command;
use std::time::Duration;

fn fixture() -> Command {
    let mut command = Command::new("/bin/sh");
    command.args([
        "-c",
        "printf '%s\\n' '{\"event\":\"ready\",\"tunnel_profile_id\":\"spoofed\"}'; cat >/dev/null",
    ]);
    command
}

async fn start(supervisor: &mut ProcessSupervisor, key: ProcessKey) -> u32 {
    let mut events = supervisor
        .spawn_owned(key, fixture(), true)
        .await
        .unwrap()
        .unwrap();
    let ready = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ready["event"], "ready");
    assert_eq!(
        ready["tunnel_profile_id"],
        serde_json::json!(key.tunnel_profile_id().unwrap())
    );
    let snapshot = supervisor.snapshot(key).unwrap();
    assert_eq!(snapshot.phase, ProcessPhase::Running);
    snapshot.pid.unwrap()
}

#[tokio::test]
async fn profiles_run_stop_restart_and_shutdown_independently() {
    let activity = ActivityLog::default();
    let mut supervisor = ProcessSupervisor::new(activity.clone());
    let a = ProcessKey::RegularTunnel(TunnelProfileId::new());
    let b = ProcessKey::RegularTunnel(TunnelProfileId::new());
    let c = ProcessKey::RegularTunnel(TunnelProfileId::new());
    let a_pid = start(&mut supervisor, a).await;
    let b_pid = start(&mut supervisor, b).await;
    let c_pid = start(&mut supervisor, c).await;
    assert_ne!(a_pid, b_pid);
    assert_ne!(b_pid, c_pid);
    assert_eq!(supervisor.keys().len(), 3);
    assert!(supervisor.spawn_owned(a, fixture(), true).await.is_err());
    assert_eq!(supervisor.snapshot(a).unwrap().pid, Some(a_pid));

    supervisor.stop_checked(a).await.unwrap();
    assert!(supervisor.snapshot(a).is_none());
    assert_eq!(supervisor.snapshot(b).unwrap().pid, Some(b_pid));
    assert_eq!(supervisor.snapshot(c).unwrap().phase, ProcessPhase::Running);
    let a_pid = start(&mut supervisor, a).await;
    supervisor.stop_checked(b).await.unwrap();
    let new_b_pid = start(&mut supervisor, b).await;
    assert_ne!(b_pid, new_b_pid);
    assert_eq!(supervisor.snapshot(a).unwrap().pid, Some(a_pid));
    assert_eq!(supervisor.snapshot(c).unwrap().pid, Some(c_pid));

    supervisor.stop_all().await;
    assert!(supervisor.keys().is_empty());
    for key in [a, b, c] {
        assert!(activity
            .snapshot()
            .iter()
            .any(|entry| entry.tunnel_profile_id == key.tunnel_profile_id()));
    }
    for pid in [a_pid, new_b_pid, c_pid] {
        assert_eq!(
            unsafe { libc::kill(pid as i32, 0) },
            -1,
            "owned process survived shutdown"
        );
    }
}

#[tokio::test]
async fn failed_profile_does_not_change_another_process_generation() {
    let mut supervisor = ProcessSupervisor::new(ActivityLog::default());
    let a = ProcessKey::RegularTunnel(TunnelProfileId::new());
    let b = ProcessKey::RegularTunnel(TunnelProfileId::new());
    let a_pid = start(&mut supervisor, a).await;
    let mut failing = Command::new("/bin/sh");
    failing.args(["-c", "exit 7"]);
    let mut events = supervisor
        .spawn_owned(b, failing, true)
        .await
        .unwrap()
        .unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .is_none());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while supervisor.snapshot(b).unwrap().phase != ProcessPhase::Failed {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(supervisor.snapshot(a).unwrap().pid, Some(a_pid));
    assert_eq!(supervisor.snapshot(a).unwrap().phase, ProcessPhase::Running);
    supervisor.stop_all().await;
}
