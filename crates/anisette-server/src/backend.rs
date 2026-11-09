use crate::store::{HeaderIdentity, Store};
use anisette::provisioning;
use anisette::{
    protocol::{ClientInfo, DSID, Identifier, ServerMessage, V1Headers},
    session::ProvisioningContext,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use clap::ValueEnum;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread,
};
use tokio::sync::oneshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Backend {
    Android,
    Macos,
    Windows,
}

impl Backend {
    const fn index(self) -> usize {
        match self {
            Self::Android => 0,
            Self::Macos => 1,
            Self::Windows => 2,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Android => "android",
            Self::Macos => "macos",
            Self::Windows => "windows",
        }
    }

    /// Routing applied when a standard V3 client finishes provisioning without the
    /// `X-Apple-I-MD-RINFO` extension. Standard clients (AltStore, SideStore, omnisette)
    /// only forward `ptm` and `tk`, so without a fallback they cannot use this backend.
    /// macOS has no verified value and still requires the extension.
    pub const fn standard_v3_routing(self) -> Option<u64> {
        match self {
            Self::Android => Some(ANDROID_V3_ROUTING),
            // Value Apple returns in `midFinishProvisioning` for Windows-profile provisioning.
            Self::Windows => Some(WINDOWS_V3_ROUTING),
            Self::Macos => None,
        }
    }

    pub fn default_binary_path(self) -> &'static Path {
        Path::new(match self {
            Self::Android => "/assets",
            Self::Macos => "/assets/CoreADI-26.x86_64",
            Self::Windows => "/assets/CoreADI.dll",
        })
    }

    pub const fn profile(self) -> ClientInfo<'static> {
        match self {
            Self::Macos => ClientInfo {
                client_info: adi_macos::provisioning::CLIENT_INFO,
                user_agent: adi_macos::provisioning::USER_AGENT,
            },
            // Android uses the Windows HTTP profile; its ADI binary remains Android.
            Self::Android => ClientInfo {
                client_info: "<PC> <Windows;6.2(0,0);9200> <com.apple.AuthKit/1 (com.apple.akd/1.0)>",
                user_agent: "akd/1.0 CFNetwork/978.0.7 Darwin/18.7.0",
            },
            // Windows CoreADI is iTunes' ADI. GSA answers an `akd` client name with an HTML
            // 403 for this anisette, but accepts the iTunes AuthKit client.
            Self::Windows => ClientInfo {
                client_info: WINDOWS_CLIENT_INFO,
                user_agent: "akd/1.0 CFNetwork/978.0.7 Darwin/18.7.0",
            },
        }
    }

    pub fn load(self, path: &Path) -> Result<Assets, Box<dyn std::error::Error + Send + Sync>> {
        Ok(match self {
            Self::Macos => Assets::Macos(adi_macos::Binary::open(path)?),
            Self::Android => Assets::Android(adi_android::Libraries::open(path)?),
            Self::Windows => Assets::Windows(adi_windows::Binary::open(path)?),
        })
    }
}

impl std::str::FromStr for Backend {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "android" => Ok(Self::Android),
            "macos" => Ok(Self::Macos),
            "windows" => Ok(Self::Windows),
            _ => Err("unknown backend"),
        }
    }
}

#[derive(Debug)]
pub struct Registry {
    default: Backend,
    state_dir: PathBuf,
    assets: [Mutex<Option<Arc<Assets>>>; 3],
    v1_senders: [Mutex<Option<tokio::sync::mpsc::Sender<V1Request>>>; 3],
    v1_joins: Mutex<Option<Vec<thread::JoinHandle<()>>>>,
}

impl Registry {
    pub fn start(default_assets: Arc<Assets>, state_dir: &Path) -> Result<Arc<Self>, String> {
        let default = default_assets.backend();
        let mut assets = std::array::from_fn(|_| Mutex::new(None));
        assets[default.index()] = Mutex::new(Some(default_assets));
        let registry = Arc::new(Self {
            default,
            state_dir: state_dir.to_path_buf(),
            assets,
            v1_senders: std::array::from_fn(|_| Mutex::new(None)),
            v1_joins: Mutex::new(Some(Vec::new())),
        });
        registry.v1_sender(default)?;
        Ok(registry)
    }

    pub const fn default_backend(&self) -> Backend {
        self.default
    }

    pub fn assets(&self, backend: Backend) -> Result<Arc<Assets>, String> {
        let mut slot = self.assets[backend.index()]
            .lock()
            .map_err(|_| "registry lock poisoned")?;
        if let Some(existing) = &*slot {
            return Ok(existing.clone());
        }
        let loaded = Arc::new(
            backend
                .load(backend.default_binary_path())
                .map_err(|e| e.to_string())?,
        );
        *slot = Some(loaded.clone());
        Ok(loaded)
    }

    pub fn existing_v1_sender(
        &self,
        backend: Backend,
    ) -> Option<tokio::sync::mpsc::Sender<V1Request>> {
        self.v1_senders[backend.index()]
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
    }

    pub fn v1_sender(
        &self,
        backend: Backend,
    ) -> Result<tokio::sync::mpsc::Sender<V1Request>, String> {
        if let Some(existing) = self.existing_v1_sender(backend) {
            return Ok(existing);
        }
        let assets = self.assets(backend)?;
        let mut joins = self.v1_joins.lock().map_err(|_| "registry lock poisoned")?;
        let Some(joins) = &mut *joins else {
            return Err("server shutting down".into());
        };
        if let Some(existing) = self.existing_v1_sender(backend) {
            return Ok(existing);
        }
        let dir = v1_state_dir(&self.state_dir, self.default, backend);
        let worker = V1Worker::start(assets, &dir)?;
        *self.v1_senders[backend.index()]
            .lock()
            .map_err(|_| "registry lock poisoned")? = Some(worker.sender.clone());
        joins.push(worker.join);
        Ok(worker.sender)
    }

    pub fn shutdown(&self) -> Result<(), &'static str> {
        let joins = self
            .v1_joins
            .lock()
            .map_err(|_| "registry lock poisoned")?
            .take()
            .unwrap_or_default();
        for slot in &self.v1_senders {
            if let Ok(mut guard) = slot.lock() {
                *guard = None;
            }
        }
        for join in joins {
            join.join().map_err(|_| "V1 worker panicked")?;
        }
        Ok(())
    }
}

fn v1_state_dir(state_dir: &Path, default: Backend, backend: Backend) -> PathBuf {
    let sub_dir = state_dir.join(backend.name());
    if backend == default && !sub_dir.join("identity.json").exists() {
        state_dir.to_path_buf()
    } else {
        sub_dir
    }
}

pub type Reply = Result<Value, &'static str>;
pub type V1Request = oneshot::Sender<Reply>;
#[derive(Debug)]
struct V1Worker {
    sender: tokio::sync::mpsc::Sender<V1Request>,
    join: thread::JoinHandle<()>,
}
impl V1Worker {
    fn start(assets: Arc<Assets>, directory: &Path) -> Result<Self, String> {
        let store = Store::open(directory, assets.name()).map_err(|e| e.to_string())?;
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<V1Request>(4);
        let (ready, result) = mpsc::sync_channel(1);
        let join = thread::Builder::new()
            .name(format!("adi-v1-{}", assets.name()))
            .spawn(move || {
                let initialized = (|| -> provisioning::Result<_> {
                    let (identifier, mut runtime, headers) = match store.load()? {
                        Some((id, bytes, headers)) => {
                            (id, Device::restore(&assets, id, &bytes)?, headers)
                        }
                        None => {
                            let mut id = [0; 16];
                            getrandom::fill(&mut id).map_err(|_| "random source failed")?;
                            let runtime = Device::new(&assets, id)?;
                            let headers = HeaderIdentity {
                                device_id: Identifier::from_bytes(id).uuid(),
                                local_user_id: provisioning::local_user_id(id),
                                serial: "0".into(),
                                client_info: assets.profile().client_info.into(),
                                user_agent: assets.profile().user_agent.into(),
                            };
                            (id, runtime, headers)
                        }
                    };
                    if !runtime.is_machine_provisioned()? {
                        runtime.provision(&assets, identifier)?;
                        store.save(identifier, &runtime.export_state(identifier)?, &headers)?;
                    }
                    if !runtime.is_machine_provisioned()? || runtime.routing()? == 0 {
                        return Err("saved device is not provisioned or has no routing".into());
                    }
                    Ok((identifier, runtime, headers))
                })();
                let (identifier, mut runtime, headers) = match initialized {
                    Ok(v) => {
                        let _ = ready.send(Ok(()));
                        v
                    }
                    Err(e) => {
                        let _ = ready.send(Err(e.to_string()));
                        return;
                    }
                };
                while let Some(reply) = receiver.blocking_recv() {
                    if reply.is_closed() {
                        continue;
                    }
                    let result = (|| -> provisioning::Result<_> {
                        if !runtime.is_machine_provisioned()? {
                            runtime.provision(&assets, identifier)?;
                            store.save(identifier, &runtime.export_state(identifier)?, &headers)?;
                        }
                        let (mid, otp) = runtime.request_otp()?;
                        let routing = runtime.routing()?;
                        if routing == 0 {
                            return Err("missing routing".into());
                        }
                        let value = serde_json::to_value(V1Headers {
                            otp: &STANDARD.encode(otp),
                            machine_id: &STANDARD.encode(mid),
                            routing: &routing.to_string(),
                            local_user_id: &headers.local_user_id,
                            serial: &headers.serial,
                            device_id: &headers.device_id,
                            client_time: &provisioning::client_time()?,
                            timezone: "UTC",
                            locale: "en_US",
                            client_info: &headers.client_info,
                            user_agent: &headers.user_agent,
                        })?;
                        // Commit state before publishing the OTP, even if the HTTP client disconnected.
                        store.save(identifier, &runtime.export_state(identifier)?, &headers)?;
                        Ok(value)
                    })();
                    match result {
                        Ok(value) => {
                            let _ = reply.send(Ok(value));
                        }
                        Err(e) => {
                            eprintln!("V1 worker ({}) error: {e}", assets.name());
                            let _ = reply
                                .send(Err("V1 generation or persistence failed; restart required"));
                            // Never continue after state save or emulator failure with uncertain state.
                            break;
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        match result.recv() {
            Ok(Ok(())) => Ok(Self { sender, join }),
            Ok(Err(message)) => {
                let _ = join.join();
                Err(message)
            }
            Err(_) => {
                let _ = join.join();
                Err("V1 worker stopped during startup".into())
            }
        }
    }
}

pub fn headers(assets: &Assets, identifier: Identifier, bytes: &[u8]) -> Reply {
    let result = (|| -> provisioning::Result<Value> {
        let mut runtime = Device::restore(assets, identifier.bytes(), bytes)?;
        let routing = runtime.routing()?;
        let routing = if routing == 0 && matches!(assets, Assets::Android(_)) {
            ANDROID_V3_ROUTING
        } else {
            routing
        };
        if routing == 0 {
            return Err("missing routing".into());
        }
        let (mid, otp) = runtime.request_otp()?;
        Ok(serde_json::to_value(ServerMessage::Headers {
            otp: &STANDARD.encode(otp),
            machine_id: &STANDARD.encode(mid),
            routing: &routing.to_string(),
        })?)
    })();
    result.map_err(|_| "state restoration or OTP generation failed")
}

#[derive(Debug)]
pub struct Context {
    runtime: Device,
    identifier: [u8; 16],
    pending: Option<u32>,
}
impl Context {
    pub fn new(assets: &Assets, identifier: Identifier) -> provisioning::Result<Self> {
        Ok(Self {
            runtime: Device::new(assets, identifier.bytes())?,
            identifier: identifier.bytes(),
            pending: None,
        })
    }
}
impl ProvisioningContext for Context {
    type Error = Box<dyn std::error::Error + Send + Sync>;
    fn start(&mut self, spim: &[u8]) -> Result<Vec<u8>, Self::Error> {
        let (cpim, session) = self.runtime.start(spim)?;
        self.pending = Some(session);
        Ok(cpim)
    }
    fn finish(
        &mut self,
        ptm: &[u8],
        tk: &[u8],
        routing: Option<u64>,
    ) -> Result<Vec<u8>, Self::Error> {
        let session = self.pending.ok_or("no pending session")?;
        self.runtime.finish(session, ptm, tk)?;
        self.pending = None;
        // Native completion may populate routing; inspect it only after consuming the handle.
        let routing = match routing {
            Some(r) => r,
            // Standard V3 omits Apple routing; use the backend's fallback when it has one.
            None => match self.runtime.backend().standard_v3_routing() {
                Some(r) => r,
                None => self.runtime.routing()?,
            },
        };
        if routing == 0 {
            return Err("this backend requires Apple routing extension".into());
        }
        self.runtime.set_routing(routing)?;
        self.runtime.export_state(self.identifier)
    }
    fn cancel(&mut self) {
        if let Some(session) = self.pending.take() {
            let _ = self.runtime.cancel(session);
        }
    }
}
impl Drop for Context {
    fn drop(&mut self) {
        self.cancel();
    }
}

const ANDROID_V3_ROUTING: u64 = 17106176;
const WINDOWS_V3_ROUTING: u64 = 84215040;
const WINDOWS_CLIENT_INFO: &str =
    "<PC> <Windows;6.2(0,0);9200> <com.apple.AuthKit/1 (com.apple.iTunes/12.12.4.1)>";

#[derive(Debug)]
pub enum Assets {
    Macos(adi_macos::Binary),
    Android(adi_android::Libraries),
    Windows(adi_windows::Binary),
}
impl Assets {
    pub const fn backend(&self) -> Backend {
        match self {
            Self::Macos(_) => Backend::Macos,
            Self::Android(_) => Backend::Android,
            Self::Windows(_) => Backend::Windows,
        }
    }
    pub const fn name(&self) -> &'static str {
        self.backend().name()
    }
    pub const fn profile(&self) -> ClientInfo<'static> {
        self.backend().profile()
    }
}
#[derive(Debug)]
enum Device {
    Macos(adi_macos::Runtime),
    Android(adi_android::Runtime),
    Windows(adi_windows::Runtime),
}
impl Device {
    const fn backend(&self) -> Backend {
        match self {
            Self::Macos(_) => Backend::Macos,
            Self::Android(_) => Backend::Android,
            Self::Windows(_) => Backend::Windows,
        }
    }
    fn new(assets: &Assets, id: [u8; 16]) -> provisioning::Result<Self> {
        Ok(match assets {
            Assets::Windows(b) => {
                let mut r = adi_windows::Runtime::new(b)?;
                if r.initialize()? < 0 {
                    return Err("Windows ADI initialization failed".into());
                }
                Self::Windows(r)
            }
            Assets::Macos(b) => {
                let mut r = adi_macos::Runtime::new(b)?;
                r.boot()?;
                Self::Macos(r)
            }
            Assets::Android(b) => Self::Android(adi_android::Runtime::new(
                b,
                Identifier::from_bytes(id),
                None,
            )?),
        })
    }
    fn restore(assets: &Assets, id: [u8; 16], state: &[u8]) -> provisioning::Result<Self> {
        Ok(match assets {
            Assets::Macos(b) => Self::Macos(adi_macos::Runtime::restore(b, id, state)?),
            Assets::Windows(b) => Self::Windows(adi_windows::Runtime::restore(b, id, state)?),
            Assets::Android(b) => Self::Android(adi_android::Runtime::new(
                b,
                Identifier::from_bytes(id),
                Some(state),
            )?),
        })
    }
    fn provision(&mut self, assets: &Assets, id: [u8; 16]) -> provisioning::Result<()> {
        let (exchange, spim) = provisioning::Exchange::start(id, assets.profile())?;
        let (cpim, session) = self.start(&spim)?;
        let result = (|| -> provisioning::Result<_> {
            let finish = exchange.finish(&cpim)?;
            self.finish(session, &finish.ptm, &finish.tk)?;
            Ok(finish.routing)
        })();
        let routing = match result {
            Ok(r) => r,
            Err(e) => {
                let _ = self.cancel(session);
                return Err(e);
            }
        };
        self.set_routing(routing)?;
        if !self.is_machine_provisioned()? || self.routing()? == 0 {
            return Err("device provisioning did not complete".into());
        }
        Ok(())
    }
    fn is_machine_provisioned(&mut self) -> provisioning::Result<bool> {
        Ok(match self {
            Self::Macos(r) => r.is_machine_provisioned(DSID)?,
            Self::Android(r) => r.is_machine_provisioned()?,
            Self::Windows(r) => r.is_provisioned()?,
        })
    }
    fn routing(&mut self) -> provisioning::Result<u64> {
        Ok(match self {
            Self::Macos(r) => r.routing(DSID)?,
            Self::Android(r) => r.routing()?,
            Self::Windows(r) => r.routing()?,
        })
    }
    fn set_routing(&mut self, n: u64) -> provisioning::Result<()> {
        match self {
            Self::Macos(r) => r.set_routing(DSID, n)?,
            Self::Android(r) => r.set_routing(n)?,
            Self::Windows(r) => r.set_routing(n)?,
        };
        Ok(())
    }
    fn start(&mut self, spim: &[u8]) -> provisioning::Result<(Vec<u8>, u32)> {
        Ok(match self {
            Self::Macos(r) => r.start_provisioning(DSID, spim)?,
            Self::Android(r) => r.start_provisioning(spim)?,
            Self::Windows(r) => r.start_provisioning(spim)?,
        })
    }
    fn finish(&mut self, s: u32, p: &[u8], t: &[u8]) -> provisioning::Result<()> {
        match self {
            Self::Macos(r) => r.finish_provisioning(s, p, t)?,
            Self::Android(r) => r.finish_provisioning(s, p, t)?,
            Self::Windows(r) => r.finish_provisioning(s, p, t)?,
        };
        Ok(())
    }
    fn cancel(&mut self, s: u32) -> provisioning::Result<()> {
        match self {
            Self::Macos(r) => r.cancel_provisioning(s)?,
            Self::Android(r) => r.cancel_provisioning(s)?,
            Self::Windows(r) => r.cancel_provisioning(s)?,
        };
        Ok(())
    }
    fn request_otp(&mut self) -> provisioning::Result<(Vec<u8>, Vec<u8>)> {
        Ok(match self {
            Self::Macos(r) => r.request_otp(DSID)?,
            Self::Android(r) => r.request_otp()?,
            Self::Windows(r) => r.request_otp()?,
        })
    }
    fn export_state(&self, id: [u8; 16]) -> provisioning::Result<Vec<u8>> {
        Ok(match self {
            Self::Macos(r) => r.export_state(id)?,
            Self::Android(r) => r.export_state()?,
            Self::Windows(r) => r.export_state(id)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Assets, Backend, Context, Device, Identifier, Path, ProvisioningContext, provisioning,
        v1_state_dir,
    };

    #[test]
    fn standard_v3_routing_falls_back_for_android_and_windows_only() {
        assert_eq!(Backend::Android.standard_v3_routing(), Some(17106176));
        assert_eq!(Backend::Windows.standard_v3_routing(), Some(84215040));
        assert_eq!(Backend::Macos.standard_v3_routing(), None);
    }

    #[test]
    fn windows_profile_claims_itunes_while_android_keeps_akd() {
        let windows = Backend::Windows.profile();
        assert!(windows.client_info.starts_with("<PC> <Windows;"));
        assert!(
            windows
                .client_info
                .ends_with("(com.apple.iTunes/12.12.4.1)>")
        );
        assert!(
            Backend::Android
                .profile()
                .client_info
                .ends_with("(com.apple.akd/1.0)>")
        );
    }

    #[test]
    fn v1_state_dir_preserves_root_for_default_and_isolates_other_backends() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path();
        assert_eq!(
            v1_state_dir(state, Backend::Android, Backend::Android),
            state
        );
        assert_eq!(
            v1_state_dir(state, Backend::Android, Backend::Macos),
            state.join("macos")
        );
        assert_eq!(
            v1_state_dir(state, Backend::Android, Backend::Windows),
            state.join("windows")
        );
        let android_sub = state.join("android");
        std::fs::create_dir_all(&android_sub).unwrap();
        std::fs::write(android_sub.join("identity.json"), "{}").unwrap();
        assert_eq!(
            v1_state_dir(state, Backend::Android, Backend::Android),
            android_sub
        );
    }
    #[test]
    #[ignore = "requires macOS/Windows binaries, Apple network and permitted Unicorn JIT"]
    fn live_missing_routing_checks_completed_native_state() {
        let mac = std::env::var_os("ANISETTE_MACOS_BINARY").unwrap();
        let win = std::env::var_os("ANISETTE_WINDOWS_BINARY").unwrap();
        // Windows: a standard V3 finish (no routing extension) now uses the fallback.
        let assets = Assets::Windows(adi_windows::Binary::open(Path::new(&win)).unwrap());
        let mut id = [0; 16];
        getrandom::fill(&mut id).unwrap();
        let mut context = Context::new(&assets, Identifier::from_bytes(id)).unwrap();
        let (exchange, spim) = provisioning::Exchange::start(id, assets.profile()).unwrap();
        let cpim = context.start(&spim).unwrap();
        let finish = exchange.finish(&cpim).unwrap();
        assert!(
            !context
                .finish(&finish.ptm, &finish.tk, None)
                .unwrap()
                .is_empty()
        );
        assert!(context.pending.is_none());
        assert_eq!(context.runtime.routing().unwrap(), 84215040);
        assert_eq!(finish.routing, 84215040, "Apple's Windows routing changed");
        assert!(!context.runtime.request_otp().unwrap().1.is_empty());
        // macOS has no verified fallback and still requires the extension.
        let assets = Assets::Macos(adi_macos::Binary::open(Path::new(&mac)).unwrap());
        let mut id = [0; 16];
        getrandom::fill(&mut id).unwrap();
        let mut context = Context::new(&assets, Identifier::from_bytes(id)).unwrap();
        let (exchange, spim) = provisioning::Exchange::start(id, assets.profile()).unwrap();
        let cpim = context.start(&spim).unwrap();
        let finish = exchange.finish(&cpim).unwrap();
        assert!(context.finish(&finish.ptm, &finish.tk, None).is_err());
        assert!(context.pending.is_none());
        assert!(context.runtime.is_machine_provisioned().unwrap());
        assert_eq!(context.runtime.routing().unwrap(), 0);
        context.cancel();
        context.runtime.set_routing(finish.routing).unwrap();
        assert!(!context.runtime.request_otp().unwrap().1.is_empty());
    }
    #[test]
    #[ignore = "requires all three ANISETTE binary paths, Apple network and permitted Unicorn JIT"]
    fn live_cancel_releases_each_real_backend() {
        let path = |name| {
            std::env::var_os(name)
                .map(std::path::PathBuf::from)
                .unwrap()
        };
        let assets = [
            Assets::Android(adi_android::Libraries::open(&path("ANISETTE_ANDROID_LIBS")).unwrap()),
            Assets::Macos(adi_macos::Binary::open(&path("ANISETTE_MACOS_BINARY")).unwrap()),
            Assets::Windows(adi_windows::Binary::open(&path("ANISETTE_WINDOWS_BINARY")).unwrap()),
        ];
        for assets in assets {
            let mut id = [0; 16];
            getrandom::fill(&mut id).unwrap();
            let mut device = Device::new(&assets, id).unwrap();
            let (_, spim) = provisioning::Exchange::start(id, assets.profile()).unwrap();
            let (_, handle) = device.start(&spim).unwrap();
            device.cancel(handle).unwrap();
            assert!(!device.is_machine_provisioned().unwrap());
            // Reuse the same VM to detect stale native handles after cancellation.
            let (_, handle) = device.start(&spim).unwrap();
            device.cancel(handle).unwrap();
            assert!(!device.is_machine_provisioned().unwrap());
        }
    }
}
