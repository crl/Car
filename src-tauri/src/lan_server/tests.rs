use super::*;
use std::sync::atomic::AtomicUsize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn serves_only_built_shell_and_named_assets() {
    assert_eq!(asset_name("/"), Some("index.html"));
    assert_eq!(
        asset_name("/assets/app-Rd3a.js"),
        Some("assets/app-Rd3a.js")
    );
    for path in [
        "/desktop.js",
        "/assets/../index.html",
        "/assets/%2e%2e%2fledger.db",
        "/assets/%61pp.js",
        "/assets/nested/app.js",
        "/assets/app.js/",
        "/assets/.git.js",
        "/assets/.env",
        "/assets/NUL.js",
        "/assets/ledger.db",
        "/assets/app.js.map",
    ] {
        assert_eq!(asset_name(path), None, "{path}");
    }
}

fn endpoint(secure: bool) -> Endpoint {
    let address = Ipv4Addr::new(192, 168, 12, 42);
    let tls = secure.then(|| {
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(rustls::server::ResolvesServerCertUsingSni::new()));
        TlsAcceptor::from(Arc::new(config))
    });
    Endpoint::new(
        address,
        if secure { ADMIN_PORT } else { GUEST_PORT },
        tls,
        "https://192.168.12.42:17861".into(),
    )
}

#[test]
fn native_operations_and_variant_mutation_paths_are_unroutable() {
    for path in [
        "/api/settings",
        "/api/backup",
        "/api/restore",
        "/api/import_excel",
        "/api/export/xlsx",
        "/api/change_password",
        "/api/exit",
        "/api/lan_status",
        "/api/%63ars",
        "/api/cars/delete/",
        "/api/cars/../entries",
    ] {
        assert_eq!(classify(&Method::GET, path), None, "GET {path}");
        assert_eq!(classify(&Method::POST, path), None, "POST {path}");
    }
    for (path, resource) in [
        ("/api/cars", "cars"),
        ("/api/cars/delete", "cars/delete"),
        ("/api/drivers", "drivers"),
        ("/api/drivers/delete", "drivers/delete"),
        ("/api/entries", "entries"),
        ("/api/entries/delete", "entries/delete"),
        ("/api/categories", "categories"),
        ("/api/categories/delete", "categories/delete"),
        ("/api/addresses", "addresses"),
        ("/api/addresses/delete", "addresses/delete"),
    ] {
        assert_eq!(classify(&Method::POST, path), Some(Action::Save(resource)));
        assert_eq!(classify(&Method::GET, path), None, "GET {path}");
    }
}

#[test]
fn mutations_require_tls_and_the_exact_single_origin_and_host() {
    let admin = endpoint(true);
    let guest = endpoint(false);
    let uri = "/api/login".parse().unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        header::HOST,
        HeaderValue::from_static("192.168.12.42:17861"),
    );
    assert!(admin.check_target(&uri, &headers).is_ok());
    assert!(guest.check_target(&uri, &headers).is_err());
    assert!(admin.check_mutation(&headers).is_err());
    headers.insert(
        header::ORIGIN,
        HeaderValue::from_static("http://192.168.12.42:17860"),
    );
    assert!(admin.check_mutation(&headers).is_err());
    headers.insert(
        header::ORIGIN,
        HeaderValue::from_static("https://192.168.12.42:17861"),
    );
    assert!(admin.check_mutation(&headers).is_ok());
    assert!(guest.check_mutation(&headers).is_err());
    headers.append(
        header::ORIGIN,
        HeaderValue::from_static("https://192.168.12.42:17861"),
    );
    assert!(admin.check_mutation(&headers).is_err());
    let absolute = "https://192.168.12.42:17861/api/login".parse().unwrap();
    assert!(admin.check_target(&absolute, &headers).is_err());
    headers.append(
        header::HOST,
        HeaderValue::from_static("192.168.12.42:17861"),
    );
    assert!(admin.check_target(&uri, &headers).is_err());
}

#[test]
fn password_rotation_and_stale_login_cannot_revoke_new_sessions() {
    let access = Access::new();
    let generation = AtomicU64::new(1);
    let previous = access.issue_session(1, &generation, None).unwrap();
    assert!(access.is_authenticated(&previous, &generation));
    generation.store(2, Ordering::SeqCst);
    assert!(!access.is_authenticated(&previous, &generation));
    let current = access.issue_session(2, &generation, None).unwrap();
    assert!(access.issue_session(1, &generation, None).is_err());
    assert!(access.is_authenticated(&current, &generation));
    assert!(!Access::new().is_authenticated(&current, &generation));
}

#[test]
fn cookie_tokens_are_revoked_on_logout_and_cannot_be_ambiguous() {
    let access = Access::new();
    let generation = AtomicU64::new(0);
    let token = access.issue_session(0, &generation, None).unwrap();
    let response = cookie_response(true, &token).unwrap();
    let cookie = response.headers()[header::SET_COOKIE].to_str().unwrap();
    assert!(cookie.starts_with("__Host-car-lan="));
    assert!(cookie.contains("; Path=/;"));
    assert!(cookie.contains("; Secure; HttpOnly; SameSite=Strict"));
    assert!(!cookie.contains("Domain="));

    let mut headers = HeaderMap::new();
    let cookie = HeaderValue::from_str(&format!("{COOKIE_NAME}={token}")).unwrap();
    headers.insert(header::COOKIE, cookie.clone());
    assert_eq!(session_token(&headers), Some(token.as_str()));
    headers.append(header::COOKIE, cookie);
    assert_eq!(session_token(&headers), None);

    access.remove_session(&token, &generation).unwrap();
    assert!(!access.is_authenticated(&token, &generation));
    let response = cookie_response(false, "").unwrap();
    assert!(response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .contains("; Max-Age=0;"));
}

#[test]
fn expired_and_disabled_sessions_cannot_authenticate() {
    let access = Access::new();
    let generation = AtomicU64::new(0);
    let expired = access.issue_session(0, &generation, None).unwrap();
    access
        .lock()
        .unwrap()
        .sessions
        .get_mut(&expired)
        .unwrap()
        .expires = Instant::now();
    assert!(!access.is_authenticated(&expired, &generation));

    let valid = access.issue_session(0, &generation, None).unwrap();
    assert!(access.is_authenticated(&valid, &generation));
    access.enabled.store(false, Ordering::SeqCst);
    assert!(!access.is_authenticated(&valid, &generation));
    assert!(access.issue_session(0, &generation, None).is_err());
}

#[test]
fn login_attempts_are_throttled_per_peer_and_across_peers() {
    let access = Access::new();
    let peer = Ipv4Addr::new(192, 168, 12, 51);
    for _ in 0..5 {
        access.allow_login(peer).unwrap();
    }
    assert_eq!(
        access.allow_login(peer).unwrap_err().0,
        StatusCode::TOO_MANY_REQUESTS
    );
    for host in 100..125 {
        access
            .allow_login(Ipv4Addr::new(192, 168, 12, host))
            .unwrap();
    }
    assert_eq!(
        access
            .allow_login(Ipv4Addr::new(192, 168, 12, 125))
            .unwrap_err()
            .0,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn failed_admin_bind_releases_guest_port() {
    let address = Ipv4Addr::LOCALHOST;
    let reserved = TcpListener::bind((address, 0)).await.unwrap();
    let admin_port = reserved.local_addr().unwrap().port();
    let guest = TcpListener::bind((address, 0)).await.unwrap();
    let guest_port = guest.local_addr().unwrap().port();
    drop(guest);
    let failed = bind_pair(address, guest_port, admin_port).await;
    assert!(failed.is_err());
    let _guest = TcpListener::bind((address, guest_port)).await.unwrap();
}

fn ledger_service(password: &str) -> (Arc<Service>, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ledger.db");
    Ledger::open(&path).unwrap();
    let desktop = super::super::DesktopState {
        path,
        operations: Mutex::new(()),
        busy: AtomicUsize::new(0),
        allow_exit: AtomicBool::new(false),
        updating: AtomicBool::new(false),
        admin_password_hash: Mutex::new(super::super::password_hash(password)),
        authenticated: AtomicBool::new(false),
        session_file: directory.path().join("admin-session"),
        lan_credential_generation: AtomicU64::new(0),
    };
    let service = Service {
        state: Arc::new(desktop),
        assets: HashMap::new(),
        access: Arc::new(Access::new()),
    };
    (Arc::new(service), directory)
}

fn request(
    endpoint: &Endpoint,
    method: Method,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> Request {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, &endpoint.host)
        .header(header::ORIGIN, &endpoint.admin_url)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        builder = builder.header(header::COOKIE, format!("{COOKIE_NAME}={token}"));
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

async fn call(service: &Service, endpoint: &Endpoint, request: Request) -> Response {
    respond(request, Ipv4Addr::new(192, 168, 12, 51), endpoint, service)
        .await
        .unwrap_or_else(IntoResponse::into_response)
}

async fn read_json(response: Response) -> Value {
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn browser_data_omits_hidden_vehicle_fields_and_preserves_legacy_labels() {
    let (service, _directory) = ledger_service("test-password");
    let connection = rusqlite::Connection::open(&service.state.path).unwrap();
    connection.execute("INSERT INTO cars(name,vin,note) VALUES('Legacy vehicle','secret-old-vin','legacy note')", []).unwrap();
    connection.execute("INSERT INTO cars(name,vin,plate,note) VALUES('Unused name','secret-vin','京A67890','note')", []).unwrap();
    let guest = endpoint(false);
    let response = call(
        &service,
        &guest,
        request(&guest, Method::GET, "/api/data", None, Value::Null),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let cars = read_json(response).await["cars"].clone();
    assert_eq!(
        cars,
        json!([
            {"id":2,"plate":"京A67890","note":"note"},
            {"id":1,"plate":null,"name":"Legacy vehicle","note":"legacy note"}
        ])
    );
    assert_eq!(
        Ledger::open(&service.state.path).unwrap().data().unwrap()["cars"][0]["vin"],
        "secret-vin"
    );
}

#[tokio::test]
async fn serves_root_with_browser_policy_and_rejects_missing_build_assets() {
    let (mut service, _directory) = ledger_service("test-password");
    Arc::get_mut(&mut service).unwrap().assets.insert(
        "index.html".into(),
        WebAsset {
            bytes: Bytes::from_static(b"<!doctype html><title>Ledger</title>"),
            content_type: HeaderValue::from_static("text/html"),
        },
    );
    let guest = endpoint(false);
    let root = call(
        &service,
        &guest,
        request(&guest, Method::GET, "/", None, Value::Null),
    )
    .await;
    assert_eq!(root.status(), StatusCode::OK);
    let policy = root.headers()[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap();
    assert!(policy.contains("connect-src 'self'"));
    assert!(policy.contains("frame-ancestors 'none'"));
    let missing = call(
        &service,
        &guest,
        request(
            &guest,
            Method::GET,
            "/assets/missing-Y7CN.js",
            None,
            Value::Null,
        ),
    )
    .await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert!(read_json(missing).await["error"].is_string());
}

#[test]
fn dev_asset_names_only_lists_regular_files_from_the_built_bundle() {
    let directory = tempfile::tempdir().unwrap();
    let dist = directory.path().join("dist");
    let assets = dist.join("assets");
    fs::create_dir_all(assets.join("unlisted-nested.js")).unwrap();
    fs::write(dist.join("index.html"), "<!doctype html>").unwrap();
    fs::write(dist.join("desktop.js"), "// Unbuilt entry").unwrap();
    fs::write(assets.join("index-A12.css"), "html{} ").unwrap();
    fs::write(assets.join("index-A12.css.map"), "{}").unwrap();
    let mut names = dev_asset_names(&dist).unwrap();
    names.sort_unstable();
    assert_eq!(names, ["assets/index-A12.css", "index.html"]);
}

fn test_tls(address: Ipv4Addr) -> (TlsAcceptor, tokio_rustls::TlsConnector) {
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec![address.to_string()]).unwrap();
    let private_key = rustls::pki_types::PrivatePkcs8KeyDer::from(signing_key.serialize_der());
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let server = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert.der().clone()], private_key.into())
        .unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert.der().clone()).unwrap();
    let client = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    (Arc::new(server).into(), Arc::new(client).into())
}

async fn read_account_from_socket<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    address: SocketAddr,
) -> Value {
    timeout(IO_TIMEOUT, async {
        stream
            .write_all(format!("GET /api/account HTTP/1.1\r\nHost: {address}\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut response = Vec::new();
        loop {
            let mut data = [0; 4096];
            let bytes = stream.read(&mut data).await.unwrap();
            assert_ne!(bytes, 0, "socket closed before sending a complete response");
            response.extend_from_slice(&data[..bytes]);
            let Some(offset) = response.windows(4).position(|window| window == b"\r\n\r\n") else {
                continue;
            };
            let headers = std::str::from_utf8(&response[..offset]).unwrap();
            assert!(headers.starts_with("HTTP/1.1 200 OK\r\n"));
            assert!(headers.contains("\r\ncache-control: no-store"));
            let size = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .unwrap()
                .parse::<usize>()
                .unwrap();
            let body = &response[offset + 4..];
            if body.len() == size {
                return serde_json::from_slice(body).unwrap();
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn https_cookie_enables_ledger_writes_and_http_ignores_the_same_cookie() {
    let (service, _directory) = ledger_service("test-password");
    let admin = endpoint(true);
    let guest = endpoint(false);
    let credentials = json!({"username":"root", "password":"test-password"});
    let blocked = call(
        &service,
        &guest,
        request(
            &guest,
            Method::POST,
            "/api/login",
            None,
            credentials.clone(),
        ),
    )
    .await;
    assert_eq!(blocked.status(), StatusCode::FORBIDDEN);
    assert!(!blocked.headers().contains_key(header::SET_COOKIE));

    let login = call(
        &service,
        &admin,
        request(&admin, Method::POST, "/api/login", None, credentials),
    )
    .await;
    assert_eq!(login.status(), StatusCode::OK);
    let cookie = login.headers()[header::SET_COOKIE].to_str().unwrap();
    let token = cookie.split(';').next().unwrap().split_once('=').unwrap().1;
    let account = call(
        &service,
        &admin,
        request(
            &admin,
            Method::GET,
            "/api/account",
            Some(token),
            Value::Null,
        ),
    )
    .await;
    assert_eq!(read_json(account).await["authenticated"], true);
    let account = call(
        &service,
        &guest,
        request(
            &guest,
            Method::GET,
            "/api/account",
            Some(token),
            Value::Null,
        ),
    )
    .await;
    assert_eq!(
        read_json(account).await,
        json!({"authenticated": false, "admin_url": "https://192.168.12.42:17861"})
    );

    let car = json!({"plate":"京A12345"});
    let blocked = call(
        &service,
        &guest,
        request(&guest, Method::POST, "/api/cars", Some(token), car.clone()),
    )
    .await;
    assert_eq!(blocked.status(), StatusCode::FORBIDDEN);
    let blocked = call(
        &service,
        &admin,
        request(&admin, Method::POST, "/api/cars", None, car.clone()),
    )
    .await;
    assert_eq!(blocked.status(), StatusCode::UNAUTHORIZED);
    let saved = call(
        &service,
        &admin,
        request(&admin, Method::POST, "/api/cars", Some(token), car.clone()),
    )
    .await;
    assert_eq!(saved.status(), StatusCode::OK);
    let data = call(
        &service,
        &guest,
        request(&guest, Method::GET, "/api/data", None, Value::Null),
    )
    .await;
    assert_eq!(read_json(data).await["cars"][0]["plate"], "京A12345");

    let logout = call(
        &service,
        &admin,
        request(
            &admin,
            Method::POST,
            "/api/logout",
            Some(token),
            Value::Null,
        ),
    )
    .await;
    assert_eq!(logout.status(), StatusCode::OK);
    let blocked = call(
        &service,
        &admin,
        request(&admin, Method::POST, "/api/cars", Some(token), car),
    )
    .await;
    assert_eq!(blocked.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn password_rotation_rejects_a_write_waiting_for_the_native_ledger_lock() {
    let (service, _directory) = ledger_service("test-password");
    let session = service
        .access
        .issue_session(0, &service.state.lan_credential_generation, None)
        .unwrap();
    let admin = endpoint(true);
    let lock = service.state.operations.lock().unwrap();
    let queued = {
        let service = service.clone();
        tokio::spawn(async move {
            call(
                &service,
                &admin,
                request(
                    &admin,
                    Method::POST,
                    "/api/cars",
                    Some(&session),
                    json!({"plate":"京A56789"}),
                ),
            )
            .await
        })
    };
    timeout(IO_TIMEOUT, async {
        while service.state.busy.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    service
        .state
        .lan_credential_generation
        .fetch_add(1, Ordering::SeqCst);
    drop(lock);
    assert_eq!(queued.await.unwrap().status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        Ledger::open(&service.state.path).unwrap().data().unwrap()["cars"],
        json!([])
    );
}

#[tokio::test]
async fn http_and_trusted_tls_serve_the_account_response() {
    let address = Ipv4Addr::new(192, 168, 12, 42);
    let (service, _directory) = ledger_service("test-password");
    let guest_address = SocketAddr::from((address, GUEST_PORT));
    let admin_address = SocketAddr::from((address, ADMIN_PORT));
    let (mut guest_client, guest_stream) = tokio::io::duplex(4096);
    let guest_worker = {
        let service = service.clone();
        tokio::spawn(async move {
            serve_connection(guest_stream, address, &endpoint(false), &service).await
        })
    };
    let guest_account = read_account_from_socket(&mut guest_client, guest_address).await;
    assert_eq!(
        guest_account,
        json!({"authenticated":false, "admin_url":"https://192.168.12.42:17861"})
    );

    let (acceptor, connector) = test_tls(address);
    let (admin_client, admin_stream) = tokio::io::duplex(4096);
    let admin_worker = tokio::spawn(async move {
        let stream = acceptor.accept(admin_stream).await.unwrap();
        serve_connection(stream, address, &endpoint(true), &service).await;
    });
    let name = rustls::pki_types::ServerName::IpAddress(address.into());
    let mut admin_client = timeout(IO_TIMEOUT, connector.connect(name, admin_client))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        read_account_from_socket(&mut admin_client, admin_address).await,
        guest_account
    );
    drop(guest_client);
    drop(admin_client);
    guest_worker.await.unwrap();
    admin_worker.await.unwrap();
}

#[tokio::test]
async fn shutdown_invalidates_sessions_and_releases_both_ports() {
    let address = Ipv4Addr::LOCALHOST;
    let netmask = Ipv4Addr::new(255, 0, 0, 0);
    let (service, _directory) = ledger_service("test-password");
    let (guest, admin) = bind_pair(address, 0, 0).await.unwrap();
    let guest_address = guest.local_addr().unwrap();
    let admin_address = admin.local_addr().unwrap();
    let admin_url = format!("https://{admin_address}");
    let (shutdown, receiver) = watch::channel(false);
    let listeners = vec![
        tokio::spawn(accept_connections(
            guest,
            address,
            netmask,
            Endpoint::new(address, guest_address.port(), None, admin_url.clone()),
            service.clone(),
            receiver.clone(),
        )),
        tokio::spawn(accept_connections(
            admin,
            address,
            netmask,
            Endpoint::new(
                address,
                admin_address.port(),
                endpoint(true).tls,
                admin_url.clone(),
            ),
            service.clone(),
            receiver,
        )),
    ];
    let server = Server {
        access: service.access.clone(),
        shutdown,
        listeners,
    };
    assert!(server.is_running());
    let active = service
        .access
        .issue_session(0, &service.state.lan_credential_generation, None)
        .unwrap();
    server.stop().await;
    assert!(!service
        .access
        .is_authenticated(&active, &service.state.lan_credential_generation));
    let _rebound = bind_pair(address, guest_address.port(), admin_address.port())
        .await
        .unwrap();
}

#[test]
fn requires_peers_on_the_selected_private_network() {
    let address = Ipv4Addr::new(192, 168, 12, 42);
    let netmask = Ipv4Addr::new(255, 255, 254, 0);
    assert!(peer_in_subnet(
        address,
        netmask,
        Ipv4Addr::new(192, 168, 13, 88)
    ));
    assert!(!peer_in_subnet(
        address,
        netmask,
        Ipv4Addr::new(192, 168, 14, 38)
    ));
    assert!(!peer_in_subnet(address, netmask, Ipv4Addr::LOCALHOST));
    assert!(!peer_in_subnet(
        Ipv4Addr::new(100, 64, 12, 24),
        netmask,
        Ipv4Addr::new(100, 64, 12, 23),
    ));
}
