#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod export;
mod ledger;
mod regions;
#[cfg(test)]
mod update_tests;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

struct DesktopState {
    path: PathBuf,
    operations: Mutex<()>,
    busy: AtomicUsize,
    allow_exit: AtomicBool,
    updating: AtomicBool,
}
type Shared = Arc<DesktopState>;
struct Busy(Shared);
impl Drop for Busy {
    fn drop(&mut self) {
        self.0.busy.fetch_sub(1, Ordering::SeqCst);
    }
}
async fn task<T: Send + 'static>(
    state: Shared,
    f: impl FnOnce(&DesktopState) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    state.busy.fetch_add(1, Ordering::SeqCst);
    tauri::async_runtime::spawn_blocking(move || {
        let _busy = Busy(state.clone());
        let _lock = state
            .operations
            .lock()
            .map_err(|_| "账本忙，请重启应用".to_string())?;
        if state.updating.load(Ordering::SeqCst) {
            return Err("更新正在安装，请稍候".into());
        }
        f(&state)
    })
    .await
    .map_err(|e| e.to_string())?
}
fn picked_path(file: tauri_plugin_dialog::FilePath) -> Result<PathBuf, String> {
    file.into_path().map_err(|e| e.to_string())
}
fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}
fn save_path(app: &tauri::AppHandle, name: &str, ext: &str) -> Result<Option<PathBuf>, String> {
    app.dialog()
        .file()
        .set_file_name(name)
        .add_filter(ext, &[ext])
        .blocking_save_file()
        .map(picked_path)
        .transpose()
}
#[tauri::command]
async fn startup_status(state: tauri::State<'_, Shared>) -> Result<Value, String> {
    task(state.inner().clone(), |s| {
        if s.path.exists() {
            ledger::Ledger::open(&s.path)?;
        }
        Ok(json!({"initialized":s.path.exists()}))
    })
    .await
}
#[tauri::command]
async fn initialize_ledger(
    app: tauri::AppHandle,
    state: tauri::State<'_, Shared>,
    import_existing: bool,
) -> Result<Value, String> {
    task(state.inner().clone(), move |s| {
        if s.path.exists() {
            return Err("账本已建立，请在设置中还原备份".into());
        }
        if import_existing {
            let Some(file) = app
                .dialog()
                .file()
                .add_filter("SQLite 账本", &["db", "sqlite", "sqlite3"])
                .blocking_pick_file()
            else {
                return Ok(json!({"cancelled":true}));
            };
            let source = picked_path(file)?;
            ledger::import_new(&s.path, &source)?;
        } else {
            ledger::Ledger::open(&s.path)?;
        }
        Ok(json!({"cancelled":false}))
    })
    .await
}
#[tauri::command]
async fn read_data(state: tauri::State<'_, Shared>) -> Result<Value, String> {
    task(state.inner().clone(), |s| {
        ledger::Ledger::open(&s.path)?.data()
    })
    .await
}
#[tauri::command]
async fn save_record(
    state: tauri::State<'_, Shared>,
    resource: String,
    data: Value,
) -> Result<Value, String> {
    task(state.inner().clone(), move |s| {
        ledger::Ledger::open(&s.path)?.save(&resource, data)
    })
    .await
}
#[tauri::command]
async fn query_regions(parent: String, q: String) -> Result<Value, String> {
    Ok(
        json!({"items":regions::query(&parent,&q),"version":"2023-06-30","coverage":"大陆31省级地区（不含港澳台）"}),
    )
}
#[tauri::command]
fn read_settings(state: tauri::State<'_, Shared>) -> Value {
    json!({"database_path":state.path,"default_path":state.path,"backup_path":state.path.parent().unwrap().join("backups")})
}
#[tauri::command]
async fn backup_ledger(
    app: tauri::AppHandle,
    state: tauri::State<'_, Shared>,
) -> Result<Value, String> {
    task(state.inner().clone(), move |s| {
        let name = format!(
            "运管家备份-{}.db",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        );
        let Some(path) = save_path(&app, &name, "db")? else {
            return Ok(json!({"cancelled":true}));
        };
        if same_path(&path, &s.path) {
            return Err("不能覆盖正在使用的账本，请选择其他文件".into());
        }
        ledger::Ledger::open(&s.path)?.snapshot_to(&path)?;
        Ok(json!({"cancelled":false,"path":path}))
    })
    .await
}
#[tauri::command]
async fn restore_ledger(
    app: tauri::AppHandle,
    state: tauri::State<'_, Shared>,
) -> Result<Value, String> {
    task(state.inner().clone(),move|s|{
 let Some(file)=app.dialog().file().add_filter("SQLite 账本备份",&["db","sqlite","sqlite3"]).blocking_pick_file() else{return Ok(json!({"cancelled":true}))};
 let source=picked_path(file)?;
 if same_path(&source,&s.path){return Err("此文件就是当前账本，请选择其他备份文件".into())}
 if !app.dialog().message("还原将替换当前车辆、账目、类型和常用地区。系统会先校验文件，并备份当前账本。确定继续？").title("还原账本").kind(MessageDialogKind::Warning).buttons(MessageDialogButtons::OkCancelCustom("还原".into(),"取消".into())).blocking_show(){return Ok(json!({"cancelled":true}))}
 let safety=ledger::Ledger::open(&s.path)?.restore_from(&source)?;
 Ok(json!({"cancelled":false,"path":s.path,"safety_backup":safety}))
}).await
}
#[tauri::command]
async fn export_ledger(
    app: tauri::AppHandle,
    state: tauri::State<'_, Shared>,
    ids: Vec<i64>,
    filename: String,
) -> Result<Value, String> {
    task(state.inner().clone(), move |s| {
        let rows = ledger::Ledger::open(&s.path)?.export_rows(&ids)?;
        if rows.is_empty() {
            return Err("当前没有可导出的收支明细，请调整筛选或先记账".into());
        }
        // Only the basename is used as a save-dialog suggestion.
        let name = filename.replace(['/', '\\', ':'], "-");
        let Some(path) = save_path(&app, &name, "xlsx")? else {
            return Ok(json!({"cancelled":true}));
        };
        if same_path(&path, &s.path) {
            return Err("不能覆盖正在使用的账本".into());
        }
        let temp = tempfile::NamedTempFile::new_in(path.parent().ok_or("保存路径无效")?)
            .map_err(|e| e.to_string())?;
        export::write_workbook(temp.path(), &rows)?;
        temp.persist(&path).map_err(|e| e.to_string())?;
        Ok(json!({"cancelled":false,"path":path}))
    })
    .await
}
#[tauri::command]
fn exit_app(app: tauri::AppHandle, state: tauri::State<'_, Shared>) -> Result<(), String> {
    if state.busy.load(Ordering::SeqCst) > 0 || state.updating.load(Ordering::SeqCst) {
        return Err("账本操作尚未完成，请稍后退出".into());
    }
    state.allow_exit.store(true, Ordering::SeqCst);
    app.exit(0);
    Ok(())
}
fn reserve_update(state: &DesktopState) -> Result<(), String> {
    let _lock = state.operations.try_lock().map_err(|_| "账本操作尚未完成，请稍后更新")?;
    if state.busy.load(Ordering::SeqCst) > 0 || state.updating.load(Ordering::SeqCst) {
        return Err("账本操作或更新尚未完成，请稍后更新".into());
    }
    state.updating.store(true, Ordering::SeqCst);
    Ok(())
}
fn release_update(state: &DesktopState) {
    state.allow_exit.store(false, Ordering::SeqCst);
    state.updating.store(false, Ordering::SeqCst);
}
#[tauri::command]
fn prepare_update(state: tauri::State<'_, Shared>) -> Result<(), String> {
    reserve_update(state.inner())
}
#[tauri::command]
fn cancel_update(state: tauri::State<'_, Shared>) {
    release_update(state.inner());
}
#[tauri::command]
fn restart_after_update(app: tauri::AppHandle, state: tauri::State<'_, Shared>) -> Result<(), String> {
    if !state.updating.load(Ordering::SeqCst) {
        return Err("尚未准备好重启更新".into());
    }
    // Ordinary Quit requests stay blocked throughout macOS bundle replacement.
    // Windows' updater exits directly after launching its installer.
    state.allow_exit.store(true, Ordering::SeqCst);
    app.restart();
}
fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                window.set_title(&format!("运管家 v{}", app.package_info().version))?;
            }
            let folder = app.path().app_data_dir()?;
            #[cfg(debug_assertions)]
            let folder = std::env::var_os("CAR_DESKTOP_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or(folder);
            std::fs::create_dir_all(&folder)?;
            app.manage(Arc::new(DesktopState {
                path: folder.join("cars.db"),
                operations: Mutex::new(()),
                busy: AtomicUsize::new(0),
                allow_exit: AtomicBool::new(false),
                updating: AtomicBool::new(false),
            }));
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let state = window.state::<Shared>();
                if state.busy.load(Ordering::SeqCst) == 0 {
                    let _ = window.emit("request-app-close", ());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            startup_status,
            initialize_ledger,
            read_data,
            save_record,
            query_regions,
            read_settings,
            backup_ledger,
            restore_ledger,
            export_ledger,
            exit_app,
            prepare_update,
            cancel_update,
            restart_after_update
        ])
        .build(tauri::generate_context!())
        .expect("运管家桌面环境初始化失败");
    app.run(|handle, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            let state = handle.state::<Shared>();
            if !state.allow_exit.load(Ordering::SeqCst) {
                api.prevent_exit();
                if state.busy.load(Ordering::SeqCst) == 0 {
                    let _ = handle.emit("request-app-close", ());
                }
            }
        }
    });
}
