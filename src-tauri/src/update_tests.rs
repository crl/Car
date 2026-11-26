use super::*;

#[test]
fn update_reservation_rejects_active_work_and_blocks_new_ledger_tasks() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(DesktopState {
        path: dir.path().join("ledger.db"),
        operations: Mutex::new(()),
        busy: AtomicUsize::new(1),
        allow_exit: AtomicBool::new(false),
        updating: AtomicBool::new(false),
        admin_password_hash: Mutex::new(password_hash("root")),
        lan_credential_generation: AtomicU64::new(0),
        authenticated: AtomicBool::new(false),
        session_file: dir.path().join("admin-session"),
    });
    assert!(reserve_update(&state).is_err());
    assert!(!state.allow_exit.load(Ordering::SeqCst));
    state.busy.store(0, Ordering::SeqCst);
    reserve_update(&state).unwrap();
    assert!(!state.allow_exit.load(Ordering::SeqCst), "user Quit must remain blocked during installation");
    assert!(reserve_update(&state).is_err());
    assert!(tauri::async_runtime::block_on(task(state.clone(), |_| Ok(42))).is_err());
    assert!(!state.path.exists());
    release_update(&state);
    assert!(!state.allow_exit.load(Ordering::SeqCst));
    assert_eq!(tauri::async_runtime::block_on(task(state, |_| Ok(42))).unwrap(), 42);
}
