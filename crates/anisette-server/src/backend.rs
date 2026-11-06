use crate::store::{HeaderIdentity, Store};
use anisette::provisioning;
use anisette::{
    protocol::{ClientInfo, DSID, Identifier, ServerMessage, V1Headers},
    session::ProvisioningContext,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;
use std::{
    path::Path,
    sync::{Arc, mpsc},
    thread,
};
use tokio::sync::oneshot;

pub type Reply = Result<Value, &'static str>;
pub type V1Request = oneshot::Sender<Reply>;
#[derive(Debug)]
pub struct V1Worker {
    pub sender: tokio::sync::mpsc::Sender<V1Request>,
    pub join: thread::JoinHandle<()>,
}
impl V1Worker {
    pub fn start(assets: Arc<Assets>, directory: &Path) -> Result<Self, String> {
        let store = Store::open(directory, assets.name()).map_err(|e| e.to_string())?;
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<V1Request>(4);
        let (ready, result) = mpsc::sync_channel(1);
        let join = thread::Builder::new()
            .name("adi-v1".into())
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
                            eprintln!("V1 worker error: {e}");
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
            // Standard V3 omits Apple routing; preserve Dadoum's Android fallback when unset.
            None if matches!(self.runtime, Device::Android(_)) => ANDROID_V3_ROUTING,
            None => self.runtime.routing()?,
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

#[derive(Debug)]
pub enum Assets {
    Macos(adi_macos::Binary),
    Android(adi_android::Libraries),
    Windows(adi_windows::Binary),
}
impl Assets {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Macos(_) => "macos",
            Self::Android(_) => "android",
            Self::Windows(_) => "windows",
        }
    }
    pub fn profile(&self) -> ClientInfo<'static> {
        match self {
            Self::Macos(_) => ClientInfo {
                client_info: adi_macos::provisioning::CLIENT_INFO,
                user_agent: adi_macos::provisioning::USER_AGENT,
            },
            // Android uses the Windows HTTP profile; its ADI binary remains Android.
            Self::Android(_) | Self::Windows(_) => ClientInfo {
                client_info: "<PC> <Windows;6.2(0,0);9200> <com.apple.AuthKit/1 (com.apple.akd/1.0)>",
                user_agent: "akd/1.0 CFNetwork/978.0.7 Darwin/18.7.0",
            },
        }
    }
}
#[derive(Debug)]
enum Device {
    Macos(adi_macos::Runtime),
    Android(adi_android::Runtime),
    Windows(adi_windows::Runtime),
}
impl Device {
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
    use super::{Assets, Context, Device, Identifier, Path, ProvisioningContext, provisioning};
    #[test]
    #[ignore = "requires macOS/Windows binaries, Apple network and permitted Unicorn JIT"]
    fn live_missing_routing_checks_completed_native_state() {
        let mac = std::env::var_os("ANISETTE_MACOS_BINARY").unwrap();
        let win = std::env::var_os("ANISETTE_WINDOWS_BINARY").unwrap();
        for assets in [
            Assets::Macos(adi_macos::Binary::open(Path::new(&mac)).unwrap()),
            Assets::Windows(adi_windows::Binary::open(Path::new(&win)).unwrap()),
        ] {
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
