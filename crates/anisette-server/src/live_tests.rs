//! Explicit device-only integration test; uses temporary identities and never prints secrets.
use super::{Arc, backend, http, store};
use anisette::provisioning::{Exchange, FinishData};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::{net::TcpListener, thread, time::Duration};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::{self, Message};

struct Server {
    url: String,
    stop: watch::Sender<bool>,
    task: Option<thread::JoinHandle<()>>,
    registry: Arc<backend::Registry>,
}
impl Server {
    fn start(binary: Arc<backend::Assets>, path: &std::path::Path) -> Self {
        let registry = backend::Registry::start(binary, path).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (stop, stopping) = watch::channel(false);
        let app = http::App::new(registry.clone(), stopping);
        let server_stop = stop.clone();
        let task = thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .max_blocking_threads(8)
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                http::serve(listener, app, server_stop).await.unwrap();
            });
        });
        Self {
            url,
            stop,
            task: Some(task),
            registry,
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
        let _ = self.registry.shutdown();
    }
}
fn socket(
    server: &Server,
) -> tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>> {
    let (socket, _) = tungstenite::connect(format!(
        "{}/v3/provisioning_session",
        server.url.replace("http://", "ws://")
    ))
    .unwrap();
    if let tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_ref() {
        stream
            .set_read_timeout(Some(Duration::from_secs(40)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
    }
    socket
}
fn read(
    socket: &mut tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>,
) -> Value {
    serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap()
}
fn send(
    socket: &mut tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>,
    value: Value,
) {
    socket
        .send(Message::Text(value.to_string().into()))
        .unwrap();
}

#[test]
#[ignore = "requires ANISETTE_MACOS_BINARY, Apple network access and permitted Unicorn JIT"]
fn live_http_restart_v3_restore_and_extended_provisioning() {
    let path = std::env::var_os("ANISETTE_MACOS_BINARY").expect("set binary path explicitly");
    let binary = Arc::new(backend::Assets::Macos(
        adi_macos::Binary::open(std::path::Path::new(&path)).unwrap(),
    ));
    run_live(binary, |finish| {
        (
            json!({
                "ptm": STANDARD.encode(&finish.ptm),
                "tk": STANDARD.encode(&finish.tk),
                "X-Apple-I-MD-RINFO": finish.routing.to_string(),
            }),
            finish.routing,
        )
    });
}

#[test]
#[ignore = "requires ANISETTE_ANDROID_LIBS, Apple network and permitted Unicorn JIT"]
fn live_android_http_restart_and_standard_v3() {
    let path = std::env::var_os("ANISETTE_ANDROID_LIBS").expect("set Android directory explicitly");
    let assets = Arc::new(backend::Assets::Android(
        adi_android::Libraries::open(std::path::Path::new(&path)).unwrap(),
    ));
    run_live(assets, |finish| {
        assert_eq!(finish.routing, 17106176, "Apple's Android routing changed");
        (
            json!({
                "ptm": STANDARD.encode(&finish.ptm),
                "tk": STANDARD.encode(&finish.tk),
            }),
            17106176,
        )
    });
}

#[test]
#[ignore = "requires ANISETTE_WINDOWS_BINARY, Apple network and permitted Unicorn JIT"]
fn live_windows_http_restart_and_standard_v3() {
    let path = std::env::var_os("ANISETTE_WINDOWS_BINARY").expect("set Windows binary explicitly");
    let assets = Arc::new(backend::Assets::Windows(
        adi_windows::Binary::open(std::path::Path::new(&path)).unwrap(),
    ));
    run_live(assets, |finish| {
        assert_eq!(finish.routing, 84215040, "Apple's Windows routing changed");
        (
            json!({
                "ptm": STANDARD.encode(&finish.ptm),
                "tk": STANDARD.encode(&finish.tk),
            }),
            84215040,
        )
    });
}

fn run_live(binary: Arc<backend::Assets>, build_end: impl FnOnce(&FinishData) -> (Value, u64)) {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("device");
    let client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(40))
        .build()
        .unwrap();
    let server = Server::start(binary.clone(), &directory);
    let profile: Value = client
        .get(format!("{}/v3/client_info", server.url))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(profile["client_info"], binary.profile().client_info);
    assert_eq!(profile["user_agent"], binary.profile().user_agent);
    let response = client.get(&server.url).send().unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let first: Value = response.json().unwrap();
    assert!(
        first["X-Apple-I-MD"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
    );
    let next: Value = client.get(&server.url).send().unwrap().json().unwrap();
    assert_eq!(first["X-Apple-I-MD-M"], next["X-Apple-I-MD-M"]);
    assert_eq!(first["X-Mme-Device-Id"], next["X-Mme-Device-Id"]);
    drop(server);
    let identity_path = directory.join("identity.json");
    let mut legacy: Value =
        serde_json::from_slice(&std::fs::read(&identity_path).unwrap()).unwrap();
    legacy["headers"]["client_info"] = json!("<Legacy>");
    legacy["headers"]["user_agent"] = json!("legacy/1.0");
    std::fs::write(&identity_path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let server = Server::start(binary.clone(), &directory);
    let restored: Value = client.get(&server.url).send().unwrap().json().unwrap();
    assert_eq!(first["X-Apple-I-MD-M"], restored["X-Apple-I-MD-M"]);
    assert_eq!(first["X-Mme-Device-Id"], restored["X-Mme-Device-Id"]);
    assert_eq!(restored["X-MMe-Client-Info"], binary.profile().client_info);
    assert_eq!(restored["User-Agent"], binary.profile().user_agent);

    let record: Value = serde_json::from_slice(&std::fs::read(&identity_path).unwrap()).unwrap();
    let identifier: [u8; 16] = serde_json::from_value(record["identifier"].clone()).unwrap();
    let request = json!({"identifier":STANDARD.encode(identifier),"adi_pb":record["adi_pb"]});
    for _ in 0..2 {
        let response: Value = client
            .post(format!("{}/v3/get_headers", server.url))
            .json(&request)
            .send()
            .unwrap()
            .json()
            .unwrap();
        assert_eq!(response["result"], "Headers");
        assert_eq!(response["X-Apple-I-MD-M"], first["X-Apple-I-MD-M"]);
        assert_eq!(response["X-Apple-I-MD-RINFO"], first["X-Apple-I-MD-RINFO"]);
    }
    let bad = client
        .post(format!("{}/v3/get_headers", server.url))
        .body("{}")
        .send()
        .unwrap();
    assert_eq!(bad.status(), 200);
    assert_eq!(bad.json::<Value>().unwrap()["result"], "GetHeadersError");
    let large = client
        .post(format!("{}/v3/get_headers", server.url))
        .body(vec![b' '; anisette::protocol::MAX_MESSAGE_BYTES + 1])
        .send()
        .unwrap();
    assert_eq!(large.status(), 413);

    let mut sessions = vec![];
    for _ in 0..4 {
        let mut ws = socket(&server);
        assert_eq!(read(&mut ws)["result"], "GiveIdentifier");
        sessions.push(ws);
    }
    let excess = tungstenite::connect(format!(
        "{}/v3/provisioning_session",
        server.url.replace("http://", "ws://")
    ));
    assert!(matches!(excess, Err(tungstenite::Error::Http(response)) if response.status() == 503));
    for mut ws in sessions {
        send(&mut ws, json!({}));
        assert_eq!(read(&mut ws)["result"], "InvalidIdentifier");
    }

    // Standard clients omit `X-Apple-I-MD-RINFO`; extended clients forward Apple's value.
    let mut id = [0; 16];
    getrandom::fill(&mut id).unwrap();
    let (exchange, spim) = Exchange::start(id, binary.profile()).unwrap();
    let mut ws = socket(&server);
    assert_eq!(read(&mut ws)["result"], "GiveIdentifier");
    send(&mut ws, json!({"identifier":STANDARD.encode(id)}));
    assert_eq!(read(&mut ws)["result"], "GiveStartProvisioningData");
    send(&mut ws, json!({"spim":STANDARD.encode(spim)}));
    let start = read(&mut ws);
    assert_eq!(start["result"], "GiveEndProvisioningData");
    let finish = exchange
        .finish(&STANDARD.decode(start["cpim"].as_str().unwrap()).unwrap())
        .unwrap();
    let (end_message, expected_routing) = build_end(&finish);
    send(&mut ws, end_message);
    let end = read(&mut ws);
    assert_eq!(end["result"], "ProvisioningSuccess");
    drop(ws);
    let request = json!({"identifier":STANDARD.encode(id),"adi_pb":end["adi_pb"]});
    for _ in 0..2 {
        let response: Value = client
            .post(format!("{}/v3/get_headers", server.url))
            .json(&request)
            .send()
            .unwrap()
            .json()
            .unwrap();
        assert_eq!(response["result"], "Headers");
        assert_eq!(
            response["X-Apple-I-MD-RINFO"]
                .as_str()
                .and_then(|s| s.parse::<u64>().ok()),
            Some(expected_routing)
        );
    }
    drop(server);
    assert!(store::Store::open(&directory, binary.name()).is_ok());
    println!(
        "Live V1/restart, V3 restore, provisioning, limits and shutdown passed; temporary state deleted; no account login tested"
    );
}
