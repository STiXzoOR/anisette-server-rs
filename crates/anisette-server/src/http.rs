use crate::backend::{self, Backend, Context, Registry};
use anisette::{
    protocol::{HeadersRequest, MAX_MESSAGE_BYTES, ServerMessage, parse_message},
    session::Session,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    Method, Request, Response as HttpResponse, StatusCode,
    body::{Bytes, Incoming},
    header::{self, HeaderValue},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use serde_json::Value;
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpListener,
    sync::{Semaphore, mpsc, oneshot, watch},
    task::JoinSet,
};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        self, Message,
        protocol::{CloseFrame, Role, WebSocketConfig, frame::coding::CloseCode},
    },
};
const CLIENT_WAIT: Duration = Duration::from_secs(3);
const EXECUTION_WAIT: Duration = Duration::from_secs(35);
type Command = (Vec<u8>, oneshot::Sender<Value>);
type Response = HttpResponse<Full<Bytes>>;
#[derive(Clone, Debug)]
pub struct App {
    pub registry: Arc<Registry>,
    pub stopping: watch::Receiver<bool>,
    v1_init: Arc<Semaphore>,
    headers: Arc<Semaphore>,
    sessions: Arc<Semaphore>,
    requests: Arc<Semaphore>,
}
impl App {
    pub fn new(registry: Arc<Registry>, stopping: watch::Receiver<bool>) -> Self {
        Self {
            registry,
            stopping,
            v1_init: Arc::new(Semaphore::new(2)),
            headers: Arc::new(Semaphore::new(4)),
            sessions: Arc::new(Semaphore::new(4)),
            requests: Arc::new(Semaphore::new(32)),
        }
    }
    pub async fn drain(&self) {
        // Permits stay in workers until native execution and cleanup actually finish.
        let _v1_init = self.v1_init.acquire_many(2).await;
        let _headers = self.headers.acquire_many(4).await;
        let _sessions = self.sessions.acquire_many(4).await;
    }
}
pub async fn serve(
    listener: TcpListener,
    app: App,
    _stop: watch::Sender<bool>,
) -> std::io::Result<()> {
    let mut stopping = app.stopping.clone();
    let mut connections = JoinSet::new();
    let result = loop {
        if *stopping.borrow() {
            break Ok(());
        }
        tokio::select! {
            _ = stopping.changed() => break Ok(()),
            Some(_) = connections.join_next(), if !connections.is_empty() => {},
            accepted = listener.accept() => {
                let (stream, _) = match accepted {
                    Ok(v) => v,
                    Err(_) => {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                };
                if connections.len() >= 64 {
                    continue;
                }
                let app = app.clone();
                let mut stopping = stopping.clone();
                connections.spawn(async move {
                    let connection = http1::Builder::new()
                        .timer(TokioTimer::new())
                        .header_read_timeout(Duration::from_secs(10))
                        .max_headers(64)
                        .serve_connection(
                            TokioIo::new(stream),
                            service_fn(move |request| handle(app.clone(), request)),
                        )
                        .with_upgrades();
                    tokio::pin!(connection);
                    if !*stopping.borrow() {
                        tokio::select! {
                            _ = &mut connection => return,
                            _ = stopping.changed() => {}
                        }
                    }
                    connection.as_mut().graceful_shutdown();
                    // A peer that stops reading must not hold process shutdown open.
                    let _ = tokio::time::timeout(Duration::from_secs(45), connection).await;
                });
            }
        }
    };
    drop(listener);
    while connections.join_next().await.is_some() {}
    app.drain().await;
    result
}
fn empty(status: StatusCode) -> Response {
    let mut response = Response::new(Full::new(Bytes::new()));
    *response.status_mut() = status;
    response
}
fn json(value: Value) -> Response {
    let mut response = Response::new(Full::new(Bytes::from(value.to_string())));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}
fn failure(message: &'static str) -> Response {
    json(
        serde_json::to_value(ServerMessage::GetHeadersError { message })
            .expect("ServerMessage serializes"),
    )
}
fn metadata(response: &mut Response) {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        "implementation-version",
        HeaderValue::from_static(concat!("anisette-server-rs/", env!("CARGO_PKG_VERSION"))),
    );
}
async fn handle(app: App, request: Request<Incoming>) -> Result<Response, Infallible> {
    let mut response = if *app.stopping.borrow() {
        empty(StatusCode::SERVICE_UNAVAILABLE)
    } else if let Ok(_permit) = app.requests.try_acquire() {
        match tokio::time::timeout(Duration::from_secs(40), route(app.clone(), request)).await {
            Ok(response) => response,
            Err(_) => empty(StatusCode::REQUEST_TIMEOUT),
        }
    } else {
        empty(StatusCode::SERVICE_UNAVAILABLE)
    };
    metadata(&mut response);
    Ok(response)
}
fn select_backend(query: Option<&str>, default: Backend) -> Result<Backend, ()> {
    let Some(query) = query.filter(|q| !q.is_empty()) else {
        return Ok(default);
    };
    let mut selected = None;
    for pair in query.split('&') {
        let Some(("backend", value)) = pair.split_once('=') else {
            return Err(());
        };
        if selected.is_some() {
            return Err(());
        }
        selected = Some(value.parse().map_err(|_| ())?);
    }
    Ok(selected.unwrap_or(default))
}
async fn route(app: App, request: Request<Incoming>) -> Response {
    let get_or_head = request.method() == Method::GET || request.method() == Method::HEAD;
    let allowed = match request.uri().path() {
        "/" | "/v3/provisioning_session" if request.method() == Method::GET => None,
        "/v3/client_info" if get_or_head => None,
        "/v3/get_headers" if request.method() == Method::POST => None,
        "/" | "/v3/provisioning_session" => Some("GET"),
        "/v3/client_info" => Some("GET, HEAD"),
        "/v3/get_headers" => Some("POST"),
        _ => return empty(StatusCode::NOT_FOUND),
    };
    if let Some(allowed) = allowed {
        let mut response = empty(StatusCode::METHOD_NOT_ALLOWED);
        response
            .headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static(allowed));
        return response;
    }
    let Ok(backend) = select_backend(request.uri().query(), app.registry.default_backend()) else {
        return empty(StatusCode::BAD_REQUEST);
    };
    match request.uri().path() {
        "/" => v1(&app, backend).await,
        "/v3/client_info" => {
            json(serde_json::to_value(backend.profile()).expect("ClientInfo serializes"))
        }
        "/v3/get_headers" => {
            let collected = tokio::time::timeout(
                CLIENT_WAIT,
                Limited::new(request.into_body(), MAX_MESSAGE_BYTES).collect(),
            )
            .await;
            let bytes = match collected {
                Ok(Ok(body)) => body.to_bytes(),
                Ok(Err(e)) => {
                    return empty(
                        if e.downcast_ref::<http_body_util::LengthLimitError>()
                            .is_some()
                        {
                            StatusCode::PAYLOAD_TOO_LARGE
                        } else {
                            StatusCode::BAD_REQUEST
                        },
                    );
                }
                Err(_) => return empty(StatusCode::REQUEST_TIMEOUT),
            };
            headers(app, backend, &bytes).await
        }
        _ => provision(app, backend, request),
    }
}
async fn v1(app: &App, backend: Backend) -> Response {
    let sender = if let Some(sender) = app.registry.existing_v1_sender(backend) {
        Some(sender)
    } else {
        let Ok(permit) = app.v1_init.clone().try_acquire_owned() else {
            return empty(StatusCode::SERVICE_UNAVAILABLE);
        };
        let registry = app.registry.clone();
        let task = spawn_worker(permit, move || registry.v1_sender(backend));
        tokio::time::timeout(EXECUTION_WAIT, task)
            .await
            .ok()
            .and_then(Result::ok)
            .and_then(Result::ok)
    };
    if let Some(sender) = sender {
        let (reply, receive) = oneshot::channel();
        if sender.try_send(reply).is_err() {
            return empty(StatusCode::SERVICE_UNAVAILABLE);
        }
        if let Ok(Ok(Ok(value))) = tokio::time::timeout(EXECUTION_WAIT, receive).await {
            return json(value);
        }
    }
    let mut response = failure("V1 worker unavailable");
    *response.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
    response
}
fn spawn_worker<T: Send + 'static>(
    permit: tokio::sync::OwnedSemaphorePermit,
    work: impl FnOnce() -> T + Send + 'static,
) -> tokio::task::JoinHandle<T> {
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work()
    })
}
fn decode_headers(
    bytes: &[u8],
) -> Result<(anisette::protocol::Identifier, Vec<u8>), anisette::protocol::InputError> {
    parse_message::<HeadersRequest>(bytes)?.decode()
}
async fn headers(app: App, backend: Backend, bytes: &[u8]) -> Response {
    let (identifier, state) = match decode_headers(bytes) {
        Ok(v) => v,
        Err(_) => return failure("invalid identifier or state"),
    };
    let Ok(permit) = app.headers.try_acquire_owned() else {
        return empty(StatusCode::SERVICE_UNAVAILABLE);
    };
    let task = spawn_worker(permit, move || {
        let assets = app
            .registry
            .assets(backend)
            .map_err(|_| "requested backend unavailable")?;
        backend::headers(&assets, identifier, &state)
    });
    match tokio::time::timeout(EXECUTION_WAIT, task).await {
        Ok(Ok(Ok(value))) => json(value),
        Ok(Ok(Err(message))) => failure(message),
        _ => failure("state restoration or OTP generation failed"),
    }
}
fn upgrade_response<B>(request: &Request<B>) -> Result<Response, ()> {
    if request.headers().contains_key(header::ORIGIN) {
        return Err(());
    }
    let key = request.headers().get("sec-websocket-key").ok_or(())?;
    if STANDARD.decode(key.as_bytes()).map_err(|_| ())?.len() != 16 {
        return Err(());
    }
    tungstenite::handshake::server::create_response_with_body(request, || Full::new(Bytes::new()))
        .map_err(|_| ())
}
fn websocket_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES))
        .max_write_buffer_size(MAX_MESSAGE_BYTES + 65536)
}
fn provision(app: App, backend: Backend, mut request: Request<Incoming>) -> Response {
    let response = match upgrade_response(&request) {
        Ok(r) => r,
        Err(()) => return empty(StatusCode::BAD_REQUEST),
    };
    let Ok(permit) = app.sessions.try_acquire_owned() else {
        return empty(StatusCode::SERVICE_UNAVAILABLE);
    };
    tokio::spawn(async move {
        let upgraded =
            match tokio::time::timeout(CLIENT_WAIT, hyper::upgrade::on(&mut request)).await {
                Ok(Ok(u)) => u,
                _ => return,
            };
        let socket = WebSocketStream::from_raw_socket(
            TokioIo::new(upgraded),
            Role::Server,
            Some(websocket_config()),
        )
        .await;
        let (sender, mut receiver) = mpsc::channel::<Command>(1);
        let registry = app.registry.clone();
        let task = spawn_worker(permit, move || {
            let assets = registry.assets(backend);
            let mut session = Session::<Context>::new();
            while let Some((bytes, reply)) = receiver.blocking_recv() {
                if reply.is_closed() {
                    break;
                }
                let Some(value) = session.receive(&bytes, |id| {
                    let assets = assets.as_ref().map_err(|_| "backend unavailable")?;
                    Context::new(assets, id)
                }) else {
                    break;
                };
                if reply.send(value).is_err() || session.is_closed() {
                    break;
                }
            }
        });
        conversation(socket, sender, app.stopping).await;
        let _ = task.await;
    });
    response
}
async fn send<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    value: Value,
) -> bool {
    matches!(
        tokio::time::timeout(
            CLIENT_WAIT,
            socket.send(Message::Text(value.to_string().into()))
        )
        .await,
        Ok(Ok(()))
    )
}
async fn conversation<S: AsyncRead + AsyncWrite + Unpin>(
    mut socket: WebSocketStream<S>,
    sender: mpsc::Sender<Command>,
    mut stopping: watch::Receiver<bool>,
) {
    if *stopping.borrow() || !send(&mut socket, Session::<Context>::initial_message()).await {
        return;
    }
    let timeout_message =
        serde_json::to_value(ServerMessage::Timeout).expect("ServerMessage serializes");
    loop {
        let input = tokio::select! {
            _ = stopping.changed() => break,
            input = tokio::time::timeout(CLIENT_WAIT, async {
                for _ in 0..16 {
                    match socket.next().await {
                        Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                        input => return input,
                    }
                }
                None
            }) => input,
        };
        let bytes = match input {
            Ok(Some(Ok(Message::Text(s)))) => s.as_bytes().to_vec(),
            Ok(Some(Ok(Message::Binary(b)))) => b.to_vec(),
            Err(_) => {
                send(&mut socket, timeout_message).await;
                break;
            }
            _ => break,
        };
        let (reply, receive) = oneshot::channel();
        if sender.try_send((bytes, reply)).is_err() {
            break;
        }
        let value = tokio::select! {
            _ = stopping.changed() => break,
            value = tokio::time::timeout(EXECUTION_WAIT, receive) => match value {
                Ok(Ok(value)) => value,
                _ => {
                    send(&mut socket, timeout_message).await;
                    break;
                }
            },
        };
        let closed = !matches!(
            value["result"].as_str(),
            Some("GiveStartProvisioningData" | "GiveEndProvisioningData")
        );
        if !send(&mut socket, value).await || closed {
            break;
        }
    }
    drop(sender);
    let _ = tokio::time::timeout(
        CLIENT_WAIT,
        socket.send(Message::Close(Some(CloseFrame {
            code: CloseCode::Normal,
            reason: "".into(),
        }))),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::{
        Arc, Backend, Command, Duration, HeaderValue, MAX_MESSAGE_BYTES, Request, Semaphore,
        StatusCode, StreamExt, TcpListener, Value, conversation, decode_headers, header, json,
        metadata, mpsc, select_backend, spawn_worker, upgrade_response, watch, websocket_config,
    };
    use futures_util::SinkExt;
    use tokio_tungstenite::{connect_async, tungstenite::Message as ClientMessage};
    #[tokio::test]
    async fn timed_out_request_keeps_capacity_until_worker_exits() {
        let slots = Arc::new(Semaphore::new(1));
        let permit = slots.clone().acquire_owned().await.unwrap();
        let (release, wait) = std::sync::mpsc::channel();
        let task = spawn_worker(permit, move || {
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(20), task)
                .await
                .is_err()
        );
        assert!(slots.clone().try_acquire_owned().is_err());
        release.send(()).unwrap();
        let _recovered = tokio::time::timeout(Duration::from_secs(2), slots.acquire())
            .await
            .unwrap()
            .unwrap();
    }
    async fn ws_message(
        socket: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> Value {
        let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        serde_json::from_str(message.to_text().unwrap()).unwrap()
    }

    #[tokio::test]
    async fn websocket_transport_with_stub_worker_closes_and_releases_channels() {
        let (stop, stopping) = watch::channel(false);
        let (events, mut drops) = mpsc::channel::<()>(8);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/ws", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let stopping = stopping.clone();
                let events = events.clone();
                tokio::spawn(async move {
                    let socket = tokio_tungstenite::accept_async_with_config(
                        stream,
                        Some(websocket_config()),
                    )
                    .await
                    .unwrap();
                    let (sender, mut receiver) = mpsc::channel::<Command>(1);
                    let worker = tokio::spawn(async move {
                        for result in [
                            "GiveStartProvisioningData",
                            "GiveEndProvisioningData",
                            "ProvisioningSuccess",
                        ] {
                            let Some((_, reply)) = receiver.recv().await else {
                                break;
                            };
                            if reply.send(serde_json::json!({"result": result})).is_err() {
                                break;
                            }
                        }
                        let _ = events.send(()).await;
                    });
                    conversation(socket, sender, stopping).await;
                    worker.await.unwrap();
                });
            }
        });
        let (mut socket, _) = connect_async(&url).await.unwrap();
        assert_eq!(ws_message(&mut socket).await["result"], "GiveIdentifier");
        for expected in [
            "GiveStartProvisioningData",
            "GiveEndProvisioningData",
            "ProvisioningSuccess",
        ] {
            socket.send(ClientMessage::Text("{}".into())).await.unwrap();
            assert_eq!(ws_message(&mut socket).await["result"], expected);
        }
        assert!(matches!(
            socket.next().await.unwrap().unwrap(),
            ClientMessage::Close(_)
        ));
        drops.recv().await.unwrap();

        let (mut socket, _) = connect_async(&url).await.unwrap();
        ws_message(&mut socket).await;
        assert_eq!(ws_message(&mut socket).await["result"], "Timeout");
        drops.recv().await.unwrap();

        let (socket, _) = connect_async(&url).await.unwrap();
        drop(socket);
        tokio::time::timeout(Duration::from_secs(5), drops.recv())
            .await
            .unwrap()
            .unwrap();

        let (mut socket, _) = connect_async(&url).await.unwrap();
        ws_message(&mut socket).await;
        stop.send(true).unwrap();
        assert!(matches!(
            socket.next().await.unwrap().unwrap(),
            ClientMessage::Close(_)
        ));
        tokio::time::timeout(Duration::from_secs(5), drops.recv())
            .await
            .unwrap()
            .unwrap();
        server.abort();
    }
    #[test]
    fn metadata_disables_caching_and_identifies_implementation() {
        let mut response = json(serde_json::json!({}));
        metadata(&mut response);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert!(
            response.headers()["implementation-version"]
                .to_str()
                .unwrap()
                .starts_with("anisette-server-rs/")
        );
    }
    #[test]
    fn websocket_handshake_validates_key_origin_and_protocol() {
        let mut request = Request::builder()
            .method("GET")
            .uri("/v3/provisioning_session")
            .header("connection", "Upgrade")
            .header("upgrade", "websocket")
            .header("sec-websocket-version", "13")
            .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
            .body(())
            .unwrap();
        let response = upgrade_response(&request).unwrap();
        assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
        assert_eq!(
            response.headers()["sec-websocket-accept"],
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
        request.headers_mut().insert(
            header::ORIGIN,
            HeaderValue::from_static("https://evil.example"),
        );
        assert!(upgrade_response(&request).is_err());
        request.headers_mut().remove(header::ORIGIN);
        request
            .headers_mut()
            .insert("sec-websocket-key", HeaderValue::from_static("bad"));
        assert!(upgrade_response(&request).is_err());
        request.headers_mut().insert(
            "sec-websocket-key",
            HeaderValue::from_static("dGhlIHNhbXBsZSBub25jZQ=="),
        );
        request.headers_mut().remove("upgrade");
        assert!(upgrade_response(&request).is_err());
    }
    #[test]
    fn body_errors_are_rejected_before_backend_execution() {
        for bytes in [
            b"{}".as_slice(),
            b"{\"identifier\":\"AA==\",\"adi_pb\":\"AA==\"}",
            &vec![0; MAX_MESSAGE_BYTES + 1],
        ] {
            assert!(decode_headers(bytes).is_err());
        }
    }
    #[test]
    fn query_parameter_selects_backend_or_rejects_invalid_values() {
        assert_eq!(select_backend(None, Backend::Android), Ok(Backend::Android));
        assert_eq!(select_backend(Some(""), Backend::Macos), Ok(Backend::Macos));
        for (query, expected) in [
            ("backend=android", Backend::Android),
            ("backend=macos", Backend::Macos),
            ("backend=windows", Backend::Windows),
        ] {
            assert_eq!(select_backend(Some(query), Backend::Android), Ok(expected));
        }
        for invalid in [
            "backend=ios",
            "backend=",
            "other=android",
            "backend=android&backend=macos",
            "backend=android&extra=1",
        ] {
            assert!(select_backend(Some(invalid), Backend::Android).is_err());
        }
    }
}
