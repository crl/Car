use crate::{
    lan_certificate,
    lan_server::{Server, ADMIN_PORT, GUEST_PORT},
    save_path, Shared,
};
use if_addrs::IfAddr;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    net::Ipv4Addr,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, State};
use tokio::sync::Mutex;

#[derive(Clone, PartialEq, Eq, Serialize)]
struct Network {
    id: String,
    name: String,
    address: Ipv4Addr,
    #[serde(skip)]
    netmask: Ipv4Addr,
}

fn available_networks() -> Result<Vec<Network>, String> {
    let interfaces = if_addrs::get_if_addrs().map_err(|error| format!("读取网卡失败：{error}"))?;
    let mut networks = BTreeMap::<String, Network>::new();
    for interface in interfaces {
        if !interface.is_oper_up() || interface.is_p2p() {
            continue;
        }
        let IfAddr::V4(address) = interface.addr else {
            continue;
        };
        if !address.ip.is_private() || address.netmask.is_unspecified() {
            continue;
        }
        #[cfg(windows)]
        let id = interface.adapter_name;
        #[cfg(not(windows))]
        let id = interface.name.clone();
        let network = Network {
            id: id.clone(),
            name: interface.name,
            address: address.ip,
            netmask: address.netmask,
        };
        let existing = networks.entry(id).or_insert_with(|| network.clone());
        if network.address < existing.address {
            *existing = network;
        }
    }
    let mut networks: Vec<_> = networks.into_values().collect();
    networks.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then(left.address.cmp(&right.address))
    });
    Ok(networks)
}

#[derive(Serialize, Deserialize)]
struct Preference<T> {
    network: Option<T>,
}

fn load_preference(path: &Path) -> Result<Option<String>, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<Preference<String>>(&bytes)
            .map(|preference| preference.network)
            .map_err(|_| "局域网共享设置损坏，请重新选择网卡".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("无法读取局域网共享设置：{error}")),
    }
}

fn save_preference(path: &Path, network: Option<&str>) -> Result<(), String> {
    (|| -> Result<(), Box<dyn std::error::Error>> {
        let directory = path.parent().ok_or("找不到共享设置目录")?;
        fs::create_dir_all(directory)?;
        let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
        serde_json::to_writer(temporary.as_file_mut(), &Preference { network })?;
        temporary.as_file().sync_all()?;
        temporary.persist(path)?;
        Ok(())
    })()
    .map_err(|error| format!("无法保存局域网共享设置：{error}"))
}

struct Active {
    network: Network,
    server: Server,
    issued_at: Instant,
}

pub(crate) struct Sharing {
    directory: PathBuf,
    preference: PathBuf,
    selected: Option<String>,
    active: Option<Active>,
    error: Option<String>,
}

impl Sharing {
    pub(crate) fn new(directory: PathBuf) -> Self {
        let preference = directory.join("lan-sharing.json");
        let (selected, error) = match load_preference(&preference) {
            Ok(network) => (network, None),
            Err(error) => (None, Some(error)),
        };
        Self {
            directory,
            preference,
            selected,
            active: None,
            error,
        }
    }

    async fn stop(&mut self) {
        if let Some(active) = self.active.take() {
            active.server.stop().await;
        }
    }

    async fn reconcile(
        &mut self,
        state: &Shared,
        app: &AppHandle,
        networks: &Result<Vec<Network>, String>,
    ) {
        let Some(selected) = self.selected.as_deref() else {
            self.stop().await;
            return;
        };
        let network = networks
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|networks| {
                networks
                    .iter()
                    .find(|network| network.id == selected)
                    .cloned()
                    .ok_or_else(|| "所选网卡未连接或没有私有 IPv4 地址".into())
            });
        let network = match network {
            Ok(network) => network,
            Err(error) => {
                self.stop().await;
                self.error = Some(error);
                return;
            }
        };
        if !state.path.is_file() {
            self.stop().await;
            self.error = Some("请先创建或导入账本".into());
            return;
        }
        if self.active.as_ref().is_some_and(|active| {
            active.network == network
                && active.server.is_running()
                && active.issued_at.elapsed() < Duration::from_secs(60 * 24 * 60 * 60)
        }) {
            return;
        }
        self.stop().await;
        let started = async {
            let certificate = lan_certificate::prepare(&self.directory, network.address)?;
            Server::start(
                state.clone(),
                app.clone(),
                network.address,
                network.netmask,
                certificate.config,
            )
            .await
        }
        .await;
        match started {
            Ok(server) => {
                self.active = Some(Active {
                    network,
                    server,
                    issued_at: Instant::now(),
                });
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn status(&self, networks: Result<Vec<Network>, String>) -> Value {
        let active = self
            .active
            .as_ref()
            .filter(|active| active.server.is_running());
        let address = active.map(|active| active.network.address);
        let (networks, adapter_error) = match networks {
            Ok(networks) => (networks, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        json!({
            "enabled": self.selected.is_some(),
            "running": active.is_some(),
            "network": self.selected,
            "networks": networks,
            "guest_url": address.map(|addr| format!("http://{addr}:{GUEST_PORT}")),
            "admin_url": address.map(|addr| format!("https://{addr}:{ADMIN_PORT}")),
            "error": self.error.as_ref().or(adapter_error.as_ref()),
            "certificate_available": lan_certificate::certificate_available(&self.directory),
        })
    }
}

pub(crate) type SharingState = Mutex<Sharing>;

pub(crate) fn watch(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let Some(state) = app.try_state::<Shared>() else {
                break;
            };
            let Some(sharing) = app.try_state::<SharingState>() else {
                break;
            };
            let mut sharing = sharing.lock().await;
            if state.allow_exit.load(Ordering::SeqCst) {
                break;
            }
            if sharing.selected.is_some() {
                sharing
                    .reconcile(state.inner(), &app, &available_networks())
                    .await;
            }
        }
    });
}

pub(crate) async fn shutdown(app: &AppHandle) {
    app.state::<SharingState>().lock().await.stop().await;
}

#[tauri::command]
pub(crate) async fn lan_status(sharing: State<'_, SharingState>) -> Result<Value, String> {
    Ok(sharing.lock().await.status(available_networks()))
}

#[tauri::command]
pub(crate) async fn set_lan_sharing(
    app: AppHandle,
    state: State<'_, Shared>,
    sharing: State<'_, SharingState>,
    enabled: bool,
    network: Option<String>,
) -> Result<Value, String> {
    let mut sharing = sharing.lock().await;
    if !state.authenticated.load(Ordering::SeqCst) {
        return Err("请先在桌面端登录管理员账号".into());
    }
    if state.allow_exit.load(Ordering::SeqCst) || state.updating.load(Ordering::SeqCst) {
        return Err("应用正在退出或更新，请稍后再试".into());
    }
    let networks = available_networks();
    let selection = if enabled {
        let selected = network.ok_or("请选择共享网卡")?;
        let networks = networks.as_ref().map_err(Clone::clone)?;
        if !networks.iter().any(|network| network.id == selected) {
            return Err("所选网卡未连接或没有私有 IPv4 地址".into());
        }
        if !state.path.is_file() {
            return Err("请先创建或导入账本".into());
        }
        Some(selected)
    } else {
        None
    };
    save_preference(&sharing.preference, selection.as_deref())?;
    sharing.selected = selection;
    sharing.error = None;
    sharing.reconcile(state.inner(), &app, &networks).await;
    Ok(sharing.status(networks))
}

#[tauri::command]
pub(crate) async fn export_lan_certificate(
    app: AppHandle,
    state: State<'_, Shared>,
    sharing: State<'_, SharingState>,
) -> Result<Value, String> {
    if !state.authenticated.load(Ordering::SeqCst) {
        return Err("请先在桌面端登录管理员账号".into());
    }
    let directory = sharing.lock().await.directory.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let certificate = lan_certificate::read_ca_certificate(&directory)?;
        let Some(path) = save_path(&app, "运管家局域网证书.cer", "cer")? else {
            return Ok(json!({"cancelled":true}));
        };
        let is_certificate = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("cer"));
        if !is_certificate {
            return Err("请选择 .cer 证书文件".into());
        }
        let folder = path.parent().ok_or("找不到证书保存目录")?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(folder).map_err(|error| error.to_string())?;
        temporary
            .write_all(&certificate)
            .map_err(|error| error.to_string())?;
        temporary.persist(path).map_err(|error| error.to_string())?;
        Ok(json!({"cancelled":false}))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sharing_is_off_until_a_network_has_been_saved() {
        let folder = tempfile::tempdir().unwrap();
        let location = folder.path().join("lan-sharing.json");
        assert_eq!(load_preference(&location).unwrap(), None);
        save_preference(&location, Some("Ethernet")).unwrap();
        assert_eq!(
            load_preference(&location).unwrap().as_deref(),
            Some("Ethernet")
        );
        save_preference(&location, None).unwrap();
        assert_eq!(load_preference(&location).unwrap(), None);
    }

    #[test]
    fn a_corrupt_preference_cannot_silently_enable_sharing() {
        let folder = tempfile::tempdir().unwrap();
        let location = folder.path().join("lan-sharing.json");
        std::fs::write(&location, b"{invalid").unwrap();
        assert!(load_preference(&location).is_err());
    }
}
