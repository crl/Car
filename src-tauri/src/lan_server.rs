use super::{ledger::Ledger, region_data, task, verify_lan_password, LanLoginError, Shared};
use axum::{
    body::{to_bytes, Body, Bytes},
    extract::{Query, Request},
    http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    Json,
};
use hyper::{server::conn::http1, service::service_fn};
use hyper_util::rt::{TokioIo, TokioTimer};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    convert::Infallible,
    fs, io,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::utils::config::FrontendDist;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpListener,
    sync::watch,
    task::{JoinHandle, JoinSet},
    time::{sleep, timeout},
};
use tokio_rustls::TlsAcceptor;

pub(crate) const GUEST_PORT: u16 = 17860;
pub(crate) const ADMIN_PORT: u16 = 17861;
const IO_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECTION_LIMIT: usize = 64;
const MAX_JSON_BYTES: usize = 256 * 1024;
const COOKIE_NAME: &str = "__Host-car-lan";
const SESSION_LIFETIME: Duration = Duration::from_secs(12 * 60 * 60);
const SESSION_LIMIT: usize = 128;
const LOGIN_WINDOW: Duration = Duration::from_secs(60);
const LOGIN_PEER_LIMIT: usize = 5;
const LOGIN_TOTAL_LIMIT: usize = 30;
const CSP: &str = "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-src 'none'; frame-ancestors 'none'; form-action 'self'";

pub(crate) struct Server {
    access: Arc<Access>,
    shutdown: watch::Sender<bool>,
    listeners: Vec<JoinHandle<()>>,
}

impl Server {
    pub(crate) async fn start(
        state: Shared,
        assets: tauri::AppHandle,
        address: Ipv4Addr,
        netmask: Ipv4Addr,
        tls: Arc<rustls::ServerConfig>,
    ) -> Result<Self, String> {
        if !peer_in_subnet(address, netmask, address) {
            return Err("请选择有效的私有 IPv4 局域网接口".into());
        }
        let assets = web_assets(&assets)?;
        let (guest, admin) = bind_pair(address, GUEST_PORT, ADMIN_PORT).await?;
        let access = Arc::new(Access::new());
        let service = Arc::new(Service {
            state,
            assets,
            access: access.clone(),
        });
        let (shutdown, receiver) = watch::channel(false);
        let admin_url = format!("https://{address}:{ADMIN_PORT}");
        let listeners = vec![
            tokio::spawn(accept_connections(
                guest,
                address,
                netmask,
                Endpoint::new(address, GUEST_PORT, None, admin_url.clone()),
                service.clone(),
                receiver.clone(),
            )),
            tokio::spawn(accept_connections(
                admin,
                address,
                netmask,
                Endpoint::new(address, ADMIN_PORT, Some(tls.into()), admin_url),
                service,
                receiver,
            )),
        ];
        Ok(Self {
            access,
            shutdown,
            listeners,
        })
    }

    pub(crate) fn is_running(&self) -> bool {
        self.listeners
            .iter()
            .all(|listener| !listener.is_finished())
    }

    pub(crate) async fn stop(mut self) {
        self.disable();
        for listener in self.listeners.drain(..) {
            let _ = listener.await;
        }
        let access = self.access.clone();
        // Writes already holding this lock are allowed to finish. Queued writes recheck enabled.
        let _ = tokio::task::spawn_blocking(move || {
            let _write = access
                .writes
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let mut state = access
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            *state = SessionState::default();
        })
        .await;
    }

    fn disable(&self) {
        self.access.enabled.store(false, Ordering::SeqCst);
        let _ = self.shutdown.send(true);
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.disable();
    }
}

async fn bind_pair(
    address: Ipv4Addr,
    guest_port: u16,
    admin_port: u16,
) -> Result<(TcpListener, TcpListener), String> {
    fn bind_error(port: u16, error: io::Error) -> String {
        if error.kind() == io::ErrorKind::AddrInUse {
            format!("局域网端口 {port} 已被占用")
        } else {
            format!("无法监听局域网端口 {port}：{error}")
        }
    }
    let guest = TcpListener::bind(SocketAddrV4::new(address, guest_port))
        .await
        .map_err(|error| bind_error(guest_port, error))?;
    let admin = TcpListener::bind(SocketAddrV4::new(address, admin_port))
        .await
        .map_err(|error| bind_error(admin_port, error))?;
    Ok((guest, admin))
}

struct WebAsset {
    bytes: Bytes,
    content_type: HeaderValue,
}

struct Service {
    state: Shared,
    assets: HashMap<String, WebAsset>,
    access: Arc<Access>,
}

fn web_assets(app: &tauri::AppHandle) -> Result<HashMap<String, WebAsset>, String> {
    let error = "未找到局域网网页资源，开发环境请先运行 npm run build";
    let resolver = app.asset_resolver();
    let config = &app.config().build;
    let names: Vec<_> = if tauri::is_dev() && config.dev_url.is_some() {
        let Some(FrontendDist::Directory(dist)) = &config.frontend_dist else {
            return Err(error.into());
        };
        dev_asset_names(&Path::new(env!("CARGO_MANIFEST_DIR")).join(dist))
            .map_err(|_| error.to_owned())?
    } else {
        resolver
            .iter()
            .filter_map(|(path, _)| {
                if path == "/index.html" {
                    Some("index.html".into())
                } else {
                    asset_name(&path).map(str::to_owned)
                }
            })
            .collect()
    };
    let assets: HashMap<_, _> = names
        .into_iter()
        .filter_map(|name| {
            let asset = resolver.get(name.clone())?;
            let content_type = HeaderValue::from_str(&asset.mime_type).ok()?;
            Some((
                name,
                WebAsset {
                    bytes: asset.bytes.into(),
                    content_type,
                },
            ))
        })
        .collect();
    if !assets.contains_key("index.html") || assets.len() <= 1 {
        return Err(error.into());
    }
    Ok(assets)
}

fn dev_asset_names(dist: &Path) -> io::Result<Vec<String>> {
    let assets = dist.join("assets");
    if !fs::symlink_metadata(dist)?.file_type().is_dir()
        || !fs::symlink_metadata(&assets)?.file_type().is_dir()
        || !fs::symlink_metadata(dist.join("index.html"))?
            .file_type()
            .is_file()
    {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut names = vec!["index.html".into()];
    for entry in fs::read_dir(assets)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            let path = format!("/assets/{}", entry.file_name().to_string_lossy());
            if let Some(name) = asset_name(&path) {
                names.push(name.into());
            }
        }
    }
    Ok(names)
}

struct Endpoint {
    host: String,
    admin_url: String,
    tls: Option<TlsAcceptor>,
}

impl Endpoint {
    fn new(address: Ipv4Addr, port: u16, tls: Option<TlsAcceptor>, admin_url: String) -> Self {
        Self {
            host: format!("{address}:{port}"),
            admin_url,
            tls,
        }
    }

    fn check_target(&self, uri: &Uri, headers: &HeaderMap) -> Result<(), ApiError> {
        if uri.authority().is_some() || !header_equals(headers, header::HOST, &self.host) {
            return Err(ApiError(StatusCode::BAD_REQUEST, "请求的主机无效"));
        }
        Ok(())
    }

    fn check_mutation(&self, headers: &HeaderMap) -> Result<(), ApiError> {
        if self.tls.is_none() {
            return Err(ApiError(
                StatusCode::FORBIDDEN,
                "请通过 HTTPS 管理入口登录或修改账本",
            ));
        }
        if !header_equals(headers, header::ORIGIN, &self.admin_url) {
            return Err(ApiError(StatusCode::FORBIDDEN, "请求的来源无效"));
        }
        Ok(())
    }
}

async fn accept_connections(
    listener: TcpListener,
    address: Ipv4Addr,
    netmask: Ipv4Addr,
    endpoint: Endpoint,
    service: Arc<Service>,
    mut shutdown: watch::Receiver<bool>,
) {
    let endpoint = Arc::new(endpoint);
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => break,
            _ = connections.join_next(), if !connections.is_empty() => {},
            accepted = listener.accept() => match accepted {
                Ok((stream, SocketAddr::V4(peer))) if connections.len() < CONNECTION_LIMIT
                    && peer_in_subnet(address, netmask, *peer.ip()) => {
                    let peer = *peer.ip();
                    let service = service.clone();
                    let endpoint = endpoint.clone();
                    connections.spawn(async move {
                        if let Some(tls) = &endpoint.tls {
                            if let Ok(Ok(stream)) = timeout(IO_TIMEOUT, tls.accept(stream)).await {
                                serve_connection(stream, peer, &endpoint, &service).await;
                            }
                        } else {
                            serve_connection(stream, peer, &endpoint, &service).await;
                        }
                    });
                }
                Ok(_) => {},
                Err(_) => sleep(Duration::from_millis(100)).await,
            },
        }
    }
    drop(listener);
    connections.shutdown().await;
}

async fn serve_connection<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    peer: Ipv4Addr,
    endpoint: &Endpoint,
    service: &Service,
) {
    let handler = service_fn(|request| async move {
        let response = respond(request.map(Body::new), peer, endpoint, service).await;
        let mut response = response.unwrap_or_else(IntoResponse::into_response);
        response.headers_mut().extend([
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
            (
                header::REFERRER_POLICY,
                HeaderValue::from_static("no-referrer"),
            ),
            (
                HeaderName::from_static("cross-origin-resource-policy"),
                HeaderValue::from_static("same-origin"),
            ),
        ]);
        Ok::<_, Infallible>(response)
    });
    let _ = timeout(
        Duration::from_secs(5 * 60),
        http1::Builder::new()
            .timer(TokioTimer::new())
            .header_read_timeout(IO_TIMEOUT)
            .serve_connection(TokioIo::new(stream), handler),
    )
    .await;
}

fn header_equals(headers: &HeaderMap, name: HeaderName, expected: &str) -> bool {
    let values = headers.get_all(name);
    let mut values = values.iter();
    values
        .next()
        .is_some_and(|value| value.as_bytes() == expected.as_bytes())
        && values.next().is_none()
}

#[derive(Debug)]
struct ApiError(StatusCode, &'static str);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

const UNAVAILABLE: ApiError = ApiError(StatusCode::SERVICE_UNAVAILABLE, "账本暂不可用，请稍后重试");
const UNAUTHENTICATED: ApiError = ApiError(StatusCode::UNAUTHORIZED, "请先以管理员身份登录");
const NOT_FOUND: ApiError = ApiError(StatusCode::NOT_FOUND, "未找到请求的资源");

#[derive(Deserialize)]
struct Login {
    username: String,
    password: String,
}

#[derive(Deserialize)]
#[serde(default)]
#[derive(Default)]
struct RegionQuery {
    parent: String,
    q: String,
    all: Option<bool>,
}

async fn json_body<T: DeserializeOwned>(request: Request) -> Result<T, ApiError> {
    let is_json = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"));
    if !is_json {
        return Err(ApiError(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "请发送 JSON 请求",
        ));
    }
    let body = timeout(IO_TIMEOUT, to_bytes(request.into_body(), MAX_JSON_BYTES))
        .await
        .map_err(|_| ApiError(StatusCode::REQUEST_TIMEOUT, "读取请求超时"))?
        .map_err(|_| ApiError(StatusCode::BAD_REQUEST, "请求体过大或无法读取"))?;
    serde_json::from_slice(&body)
        .map_err(|_| ApiError(StatusCode::BAD_REQUEST, "JSON 请求格式无效"))
}

#[derive(Debug, PartialEq, Eq)]
enum Action<'a> {
    Account,
    Data,
    Regions,
    Asset(&'a str),
    Login,
    Logout,
    Save(&'static str),
}

fn classify<'a>(method: &Method, path: &'a str) -> Option<Action<'a>> {
    match (method, path) {
        (&Method::GET, "/api/account") => Some(Action::Account),
        (&Method::GET, "/api/data") => Some(Action::Data),
        (&Method::GET, "/api/regions") => Some(Action::Regions),
        (&Method::GET, _) => asset_name(path).map(Action::Asset),
        (&Method::POST, "/api/login") => Some(Action::Login),
        (&Method::POST, "/api/logout") => Some(Action::Logout),
        (&Method::POST, "/api/cars") => Some(Action::Save("cars")),
        (&Method::POST, "/api/cars/delete") => Some(Action::Save("cars/delete")),
        (&Method::POST, "/api/drivers") => Some(Action::Save("drivers")),
        (&Method::POST, "/api/drivers/delete") => Some(Action::Save("drivers/delete")),
        (&Method::POST, "/api/entries") => Some(Action::Save("entries")),
        (&Method::POST, "/api/entries/delete") => Some(Action::Save("entries/delete")),
        (&Method::POST, "/api/entries/delete-batch") => Some(Action::Save("entries/delete-batch")),
        (&Method::POST, "/api/categories") => Some(Action::Save("categories")),
        (&Method::POST, "/api/categories/delete") => Some(Action::Save("categories/delete")),
        (&Method::POST, "/api/addresses") => Some(Action::Save("addresses")),
        (&Method::POST, "/api/addresses/delete") => Some(Action::Save("addresses/delete")),
        (&Method::POST, "/api/addresses/merge-preview") => Some(Action::Save("addresses/merge-preview")),
        (&Method::POST, "/api/addresses/merge") => Some(Action::Save("addresses/merge")),
        _ => None,
    }
}

async fn respond(
    request: Request,
    peer: Ipv4Addr,
    endpoint: &Endpoint,
    service: &Service,
) -> Result<Response, ApiError> {
    endpoint.check_target(request.uri(), request.headers())?;
    if !service.access.enabled.load(Ordering::SeqCst) {
        return Err(UNAVAILABLE);
    }
    match classify(request.method(), request.uri().path()).ok_or(NOT_FOUND)? {
        Action::Account => {
            let authenticated = endpoint.tls.is_some()
                && session_token(request.headers()).is_some_and(|token| {
                    service
                        .access
                        .is_authenticated(token, &service.state.lan_credential_generation)
                });
            Ok(
                Json(json!({ "authenticated": authenticated, "admin_url": endpoint.admin_url }))
                    .into_response(),
            )
        }
        Action::Data => {
            let access = service.access.clone();
            let data = task(service.state.clone(), move |state| {
                if !access.enabled.load(Ordering::SeqCst) || !state.path.is_file() {
                    return Err("账本尚未建立".into());
                }
                Ok(browser_data(Ledger::open(&state.path)?.data()?))
            })
            .await
            .map_err(|_| UNAVAILABLE)?;
            Ok(Json(data).into_response())
        }
        Action::Regions => {
            let Query(params) = Query::<RegionQuery>::try_from_uri(request.uri())
                .map_err(|_| ApiError(StatusCode::BAD_REQUEST, "地区查询参数无效"))?;
            let data = tokio::task::spawn_blocking(move || {
                region_data(&params.parent, &params.q, params.all.unwrap_or(false))
            })
            .await
            .map_err(|_| UNAVAILABLE)?;
            Ok(Json(data).into_response())
        }
        Action::Asset(name) => asset_response(service, name),
        Action::Login => {
            endpoint.check_mutation(request.headers())?;
            let previous = session_token(request.headers()).map(str::to_owned);
            let Login { username, password } = json_body(request).await?;
            service.access.allow_login(peer)?;
            let generation =
                verify_lan_password(&service.state, &username, &password).map_err(|error| {
                    match error {
                        LanLoginError::Invalid => {
                            ApiError(StatusCode::UNAUTHORIZED, "账号或密码错误")
                        }
                        LanLoginError::DefaultPassword => {
                            ApiError(StatusCode::FORBIDDEN, "请先在桌面端修改默认管理员密码")
                        }
                        LanLoginError::Unavailable => UNAVAILABLE,
                    }
                })?;
            let token = service.access.issue_session(
                generation,
                &service.state.lan_credential_generation,
                previous.as_deref(),
            )?;
            cookie_response(true, &token)
        }
        Action::Logout => {
            endpoint.check_mutation(request.headers())?;
            let token = session_token(request.headers()).ok_or(UNAUTHENTICATED)?;
            service
                .access
                .remove_session(token, &service.state.lan_credential_generation)?;
            cookie_response(false, "")
        }
        Action::Save(resource) => {
            endpoint.check_mutation(request.headers())?;
            let token = session_token(request.headers())
                .ok_or(UNAUTHENTICATED)?
                .to_owned();
            if !service
                .access
                .is_authenticated(&token, &service.state.lan_credential_generation)
            {
                return Err(UNAUTHENTICATED);
            }
            let data = Value::Object(json_body(request).await?);
            save_record(service, resource, data, token).await
        }
    }
}

fn browser_data(mut data: Value) -> Value {
    let mut browser = serde_json::Map::new();
    for (table, fields) in [
        ("cars", &["id", "plate", "name", "note", "color"][..]),
        ("drivers", &["id", "name", "phone", "note", "car_ids"]),
        (
            "categories",
            &["id", "kind", "name", "active", "requires_route"],
        ),
        (
            "addresses",
            &["id", "name", "detail", "active", "region_code", "aliases"],
        ),
        (
            "entries",
            &[
                "id",
                "car_id",
                "date",
                "kind",
                "category",
                "cents",
                "party",
                "note",
                "start_address",
                "end_address",
                "start_address_id",
                "end_address_id",
                "start_address_detail",
                "end_address_detail",
            ],
        ),
    ] {
        let mut records = data[table].take();
        for row in records.as_array_mut().into_iter().flatten() {
            if let Some(row) = row.as_object_mut() {
                row.retain(|key, _| fields.contains(&key.as_str()));
                if table == "cars"
                    && row
                        .get("plate")
                        .and_then(Value::as_str)
                        .is_some_and(|plate| !plate.is_empty())
                {
                    row.remove("name");
                }
            }
        }
        browser.insert(table.into(), records);
    }
    Value::Object(browser)
}

fn asset_response(service: &Service, name: &str) -> Result<Response, ApiError> {
    let asset = service.assets.get(name).ok_or(NOT_FOUND)?;
    let mut response = Body::from(asset.bytes.clone()).into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, asset.content_type.clone());
    if name == "index.html" {
        response.headers_mut().insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CSP),
        );
    }
    Ok(response)
}

async fn save_record(
    service: &Service,
    resource: &'static str,
    data: Value,
    token: String,
) -> Result<Response, ApiError> {
    let access = service.access.clone();
    task(service.state.clone(), move |desktop| {
        let _write = access.writes.lock().map_err(|_| "账本暂不可用")?;
        if !access.is_authenticated(&token, &desktop.lan_credential_generation) {
            return Ok(UNAUTHENTICATED.into_response());
        }
        if !desktop.path.is_file() {
            return Ok(UNAVAILABLE.into_response());
        }
        let ledger = Ledger::open(&desktop.path)?;
        let response = match ledger.save(resource, data) {
            Ok(value) => Json(value).into_response(),
            Err(error) => {
                (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response()
            }
        };
        Ok(response)
    })
    .await
    .map_err(|_| UNAVAILABLE)
}

fn session_token(headers: &HeaderMap) -> Option<&str> {
    let mut token = None;
    for cookie in headers.get_all(header::COOKIE) {
        let cookie = cookie.to_str().ok()?;
        for entry in cookie.split(';') {
            let Some((name, value)) = entry.trim().split_once('=') else {
                continue;
            };
            if name == COOKIE_NAME {
                if token.is_some()
                    || value.len() != 64
                    || !value.bytes().all(|c| c.is_ascii_hexdigit())
                {
                    return None;
                }
                token = Some(value);
            }
        }
    }
    token
}

fn cookie_response(authenticated: bool, token: &str) -> Result<Response, ApiError> {
    let age = if authenticated {
        SESSION_LIFETIME.as_secs()
    } else {
        0
    };
    let cookie =
        format!("{COOKIE_NAME}={token}; Path=/; Max-Age={age}; Secure; HttpOnly; SameSite=Strict");
    let mut response = Json(json!({ "authenticated": authenticated })).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(|_| UNAVAILABLE)?,
    );
    Ok(response)
}

struct Session {
    generation: u64,
    expires: Instant,
}

#[derive(Default)]
struct SessionState {
    sessions: HashMap<String, Session>,
    attempts: VecDeque<(Instant, Ipv4Addr)>,
}

impl SessionState {
    fn contains_session(&self, token: &str, generation: u64, now: Instant) -> bool {
        self.sessions
            .get(token)
            .is_some_and(|session| session.generation == generation && session.expires > now)
    }
}

struct Access {
    enabled: AtomicBool,
    writes: Mutex<()>,
    state: Mutex<SessionState>,
}

impl Access {
    fn new() -> Self {
        Self {
            enabled: AtomicBool::new(true),
            writes: Mutex::new(()),
            state: Mutex::new(SessionState::default()),
        }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, SessionState>, ApiError> {
        self.state.lock().map_err(|_| UNAVAILABLE)
    }

    fn allow_login(&self, peer: Ipv4Addr) -> Result<(), ApiError> {
        let mut state = self.lock()?;
        let now = Instant::now();
        while state
            .attempts
            .front()
            .is_some_and(|(attempt, _)| now.duration_since(*attempt) >= LOGIN_WINDOW)
        {
            state.attempts.pop_front();
        }
        if state.attempts.len() >= LOGIN_TOTAL_LIMIT
            || state.attempts.iter().filter(|(_, ip)| *ip == peer).count() >= LOGIN_PEER_LIMIT
        {
            return Err(ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "登录尝试过多，请稍后重试",
            ));
        }
        state.attempts.push_back((now, peer));
        Ok(())
    }

    fn is_authenticated(&self, token: &str, generation: &AtomicU64) -> bool {
        self.lock().is_ok_and(|state| {
            self.enabled.load(Ordering::SeqCst)
                && state.contains_session(token, generation.load(Ordering::SeqCst), Instant::now())
        })
    }

    fn issue_session(
        &self,
        generation: u64,
        current_generation: &AtomicU64,
        previous: Option<&str>,
    ) -> Result<String, ApiError> {
        use std::fmt::Write;
        let mut random = [0; 32];
        getrandom::fill(&mut random).map_err(|_| UNAVAILABLE)?;
        let mut token = String::with_capacity(64);
        for byte in random {
            write!(token, "{byte:02x}").map_err(|_| UNAVAILABLE)?;
        }
        let mut state = self.lock()?;
        if !self.enabled.load(Ordering::SeqCst) {
            return Err(UNAVAILABLE);
        }
        if generation != current_generation.load(Ordering::SeqCst) {
            return Err(UNAUTHENTICATED);
        }
        let now = Instant::now();
        state
            .sessions
            .retain(|_, session| session.expires > now && session.generation == generation);
        if let Some(previous) = previous {
            state.sessions.remove(previous);
        }
        if state.sessions.len() >= SESSION_LIMIT {
            if let Some(oldest) = state
                .sessions
                .iter()
                .min_by_key(|(_, session)| session.expires)
                .map(|(key, _)| key.clone())
            {
                state.sessions.remove(&oldest);
            }
        }
        state.sessions.insert(
            token.clone(),
            Session {
                generation,
                expires: now + SESSION_LIFETIME,
            },
        );
        Ok(token)
    }

    fn remove_session(&self, token: &str, generation: &AtomicU64) -> Result<(), ApiError> {
        let mut state = self.lock()?;
        if !self.enabled.load(Ordering::SeqCst)
            || !state.contains_session(token, generation.load(Ordering::SeqCst), Instant::now())
        {
            return Err(UNAUTHENTICATED);
        }
        state.sessions.remove(token);
        Ok(())
    }
}

fn asset_name(path: &str) -> Option<&str> {
    if path == "/" {
        return Some("index.html");
    }
    let name = path.strip_prefix("/assets/")?;
    let (stem, extension) = name.rsplit_once('.')?;
    if !matches!(
        extension,
        "js" | "css" | "woff2" | "png" | "svg" | "webp" | "ico"
    ) || !stem.contains('-')
        || !stem.starts_with(|c: char| c.is_ascii_alphanumeric())
        || !stem
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
    {
        return None;
    }
    path.strip_prefix('/')
}

fn peer_in_subnet(address: Ipv4Addr, netmask: Ipv4Addr, peer: Ipv4Addr) -> bool {
    let mask = u32::from(netmask);
    address.is_private()
        && peer.is_private()
        && mask != 0
        && (!mask).wrapping_add(1).is_power_of_two()
        && (u32::from(address) & mask) == (u32::from(peer) & mask)
}

#[cfg(test)]
#[path = "lan_server/tests.rs"]
mod tests;
