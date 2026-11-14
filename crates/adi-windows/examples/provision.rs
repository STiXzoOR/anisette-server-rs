//! Explicit device-only network smoke test. No account credentials.
use adi_windows::{Binary, Runtime};
use anisette::{protocol::ClientInfo, provisioning::Exchange};
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("provide explicit CoreADI DLL; contacts Apple")?;
    let binary = Binary::open(std::path::Path::new(&path))?;
    let mut runtime = Runtime::new(&binary)?;
    assert!(runtime.initialize()? >= 0);
    let mut id = [0; 16];
    getrandom::fill(&mut id).map_err(|_| "random source failed")?;
    let (exchange, spim) = Exchange::start(
        id,
        ClientInfo {
            client_info: adi_windows::CLIENT_INFO,
            user_agent: adi_windows::USER_AGENT,
        },
    )?;
    println!("Apple start response received");
    let (cpim, session) = runtime.start_provisioning(&spim)?;
    println!("Windows ADI produced CPIM");
    let finish = exchange.finish(&cpim)?;
    runtime.finish_provisioning(session, &finish.ptm, &finish.tk)?;
    assert!(runtime.is_provisioned()?);
    println!(
        "Windows provisioning complete; standard routing nonzero: {}",
        runtime.routing()? != 0
    );
    runtime.set_routing(finish.routing)?;
    let (mid, otp) = runtime.request_otp()?;
    assert!(!mid.is_empty() && !otp.is_empty());
    let state = runtime.export_state(id)?;
    for _ in 0..2 {
        let mut restored = Runtime::restore(&binary, id, &state)?;
        assert!(restored.is_provisioned()?);
        assert_eq!(restored.routing()?, finish.routing);
        for _ in 0..5 {
            let (next_mid, next_otp) = restored.request_otp()?;
            assert!(mid == next_mid && !next_otp.is_empty());
        }
    }
    println!("Windows OTP/two repeated restores passed; no account login tested");
    Ok(())
}
