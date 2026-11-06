//! Pinned macOS 26 client profile metadata and device provisioning orchestration.
use crate::{Binary, Runtime};
use anisette::{
    protocol::{ClientInfo, DSID},
    provisioning::{Exchange, Result},
};
pub const CLIENT_INFO: &str =
    "<MacBookPro16,2> <macOS;26.7.1;25G241> <com.apple.AuthKit/1 (com.apple.akd/1.0)>";
pub const USER_AGENT: &str = "akd/1.0 CFNetwork/3860.100.1 Darwin/25.0.0";
/// Boots and provisions a new macOS device through an Apple exchange.
///
/// # Errors
/// Returns an error if emulator startup, Apple transfers, native provisioning, routing, or final state validation fails.
pub fn provision(binary: &Binary, identifier: [u8; 16]) -> Result<Runtime> {
    let mut runtime = Runtime::new(binary)?;
    runtime.boot()?;
    let (exchange, spim) = Exchange::start(
        identifier,
        ClientInfo {
            client_info: CLIENT_INFO,
            user_agent: USER_AGENT,
        },
    )?;
    let (cpim, session) = runtime.start_provisioning(DSID, &spim)?;
    let result = (|| -> Result<u64> {
        let finish = exchange.finish(&cpim)?;
        runtime.finish_provisioning(session, &finish.ptm, &finish.tk)?;
        Ok(finish.routing)
    })();
    let routing = match result {
        Ok(routing) => routing,
        Err(error) => {
            let _ = runtime.cancel_provisioning(session);
            return Err(error);
        }
    };
    runtime.set_routing(DSID, routing)?;
    if !runtime.is_machine_provisioned(DSID)? {
        return Err("device is not provisioned".into());
    }
    Ok(runtime)
}
