use super::*;

fn test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

#[test]
fn register_and_deregister_normal() {
    let _guard = test_lock().lock().unwrap_or_else(|p| p.into_inner());
    clear_for_test();
    register(1234);
    register(5678);
    let s = snapshot();
    assert!(s.contains(&1234));
    assert!(s.contains(&5678));
    deregister(1234);
    let s = snapshot();
    assert!(!s.contains(&1234));
    assert!(s.contains(&5678));
}

#[test]
fn deregister_missing_pid_is_noop_robust() {
    let _guard = test_lock().lock().unwrap_or_else(|p| p.into_inner());
    clear_for_test();
    deregister(99999);
    assert!(snapshot().is_empty());
}

#[test]
fn registry_trace_records_counts_normal() {
    let _guard = test_lock().lock().unwrap_or_else(|p| p.into_inner());
    linkscope::trace_detail_enable();
    clear_for_test();

    register(42);
    deregister(42);

    let snapshot = linkscope::snapshot();
    assert!(snapshot.traces.iter().any(|trace| {
        trace.label == "tools.bash_processes.register.detail"
            && trace
                .fields
                .iter()
                .any(|field| field.name == "registry_size")
    }));
    assert!(snapshot.traces.iter().any(|trace| trace.label
        == "tools.bash_processes.deregister.detail"
        && trace.fields.iter().any(|field| field.name == "changed")));
}

#[test]
fn terminate_all_signals_invalid_pids_robust() {
    let _guard = test_lock().lock().unwrap_or_else(|p| p.into_inner());
    clear_for_test();
    register(4_000_000);
    let _ = terminate_all();
    assert_eq!(snapshot().len(), 1);
}

#[cfg(unix)]
#[test]
fn terminate_all_signals_detached_process_group_regression() {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let _guard = test_lock().lock().unwrap_or_else(|p| p.into_inner());
    clear_for_test();

    let dir = tempfile::tempdir().expect("tempdir");
    let marker = dir.path().join("marker.txt");
    let script = dir.path().join("child.sh");
    std::fs::write(
        &script,
        r#"trap '' TERM
(
  trap 'echo child-term >> "$MARKER"; exit 0' TERM
  while :; do sleep 1; done
) &
wait $!
"#,
    )
    .expect("write script");

    let mut command = Command::new("sh");
    command
        .arg(&script)
        .env("MARKER", &marker)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: Category 8 — FFI boundary through pre_exec. The closure only
    // calls async-signal-safe setsid() and returns last_os_error on failure.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let mut child = command.spawn().expect("spawn detached shell");
    let pid = child.id();
    register(pid);
    std::thread::sleep(Duration::from_millis(150));

    assert_eq!(terminate_all(), 1);

    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if child.try_wait().expect("try_wait").is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    if child.try_wait().expect("try_wait").is_none() {
        let _ = signal_process_tree(pid, libc::SIGKILL);
        let _ = child.wait();
        panic!("detached bash process group did not exit after terminate_all");
    }

    let marker_text = std::fs::read_to_string(&marker).expect("read marker");
    assert!(marker_text.contains("child-term"), "{marker_text:?}");
    deregister(pid);
}
