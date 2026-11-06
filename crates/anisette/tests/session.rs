//! State-machine tests with a fake backend; these do not validate Apple provisioning or OTPs.
use anisette::{
    protocol::{EndMessage, MAX_MESSAGE_BYTES},
    session::{ProvisioningContext, Session},
};
use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc};

#[derive(Default)]
struct Calls {
    start: usize,
    finish: usize,
    cancel: usize,
    drop: usize,
}
struct Fake {
    calls: Rc<RefCell<Calls>>,
    fail: &'static str,
}
impl Drop for Fake {
    fn drop(&mut self) {
        self.calls.borrow_mut().drop += 1;
    }
}
impl ProvisioningContext for Fake {
    type Error = &'static str;
    fn start(&mut self, spim: &[u8]) -> Result<Vec<u8>, Self::Error> {
        assert_eq!(spim, b"s");
        self.calls.borrow_mut().start += 1;
        match self.fail {
            "start" => Err("SECRET"),
            "output" => Ok(vec![]),
            _ => Ok(b"c".to_vec()),
        }
    }
    fn finish(
        &mut self,
        ptm: &[u8],
        tk: &[u8],
        _routing: Option<u64>,
    ) -> Result<Vec<u8>, Self::Error> {
        assert_eq!((ptm, tk), (&b"p"[..], &b"t"[..]));
        self.calls.borrow_mut().finish += 1;
        if self.fail == "finish" {
            Err("SECRET")
        } else {
            Ok(vec![0, 255])
        }
    }
    fn cancel(&mut self) {
        self.calls.borrow_mut().cancel += 1;
    }
}
fn send(
    s: &mut Session<Fake>,
    message: Value,
    calls: &Rc<RefCell<Calls>>,
    fail: &'static str,
) -> Value {
    s.receive(&serde_json::to_vec(&message).unwrap(), |_| {
        Ok::<_, ()>(Fake {
            calls: calls.clone(),
            fail,
        })
    })
    .unwrap()
}
fn begin(s: &mut Session<Fake>, calls: &Rc<RefCell<Calls>>, fail: &'static str) {
    assert_eq!(
        send(
            s,
            json!({"identifier":"ABEiM0RVZneImaq7zN3u/w=="}),
            calls,
            fail
        ),
        json!({"result":"GiveStartProvisioningData"})
    );
}
fn start(s: &mut Session<Fake>, calls: &Rc<RefCell<Calls>>) -> Value {
    send(s, json!({"spim":"cw=="}), calls, "")
}
#[test]
fn handshake_is_ordered_and_success_drops_without_cancel() {
    let calls = Rc::default();
    let mut s = Session::default();
    assert_eq!(
        Session::<Fake>::initial_message(),
        json!({"result":"GiveIdentifier"})
    );
    begin(&mut s, &calls, "");
    assert_eq!(
        start(&mut s, &calls),
        json!({"result":"GiveEndProvisioningData","cpim":"Yw=="})
    );
    assert_eq!(
        send(&mut s, json!({"ptm":"cA==","tk":"dA=="}), &calls, ""),
        json!({"result":"ProvisioningSuccess","adi_pb":"AP8="})
    );
    assert!(s.is_closed());
    assert!(
        s.receive(b"{}", |_| -> Result<Fake, ()> {
            panic!("must not create twice")
        })
        .is_none()
    );
    assert!(s.timeout().is_none());
    drop(s);
    let c = calls.borrow();
    assert_eq!((c.start, c.finish, c.cancel, c.drop), (1, 1, 0, 1));
}
#[test]
fn disconnect_and_timeout_release_each_stage() {
    for stage in 0..3 {
        for timeout in [false, true] {
            let calls = Rc::default();
            let mut s = Session::default();
            if stage > 0 {
                begin(&mut s, &calls, "");
            }
            if stage > 1 {
                start(&mut s, &calls);
            }
            if timeout {
                assert_eq!(s.timeout(), Some(json!({"result":"Timeout"})));
            }
            drop(s);
            let c = calls.borrow();
            assert_eq!(c.cancel, usize::from(stage == 2));
            assert_eq!(c.drop, usize::from(stage > 0));
        }
    }
}
#[test]
fn wrong_sequence_fails_without_backend_calls() {
    let calls = Rc::default();
    let mut s = Session::default();
    assert_eq!(
        send(&mut s, json!({"spim":"cw=="}), &calls, ""),
        json!({"result":"InvalidIdentifier"})
    );
    assert_eq!(calls.borrow().drop, 0);
    let mut s = Session::default();
    begin(&mut s, &calls, "");
    assert_eq!(
        send(&mut s, json!({"ptm":"cA==","tk":"dA=="}), &calls, "")["result"],
        "StartProvisioningError"
    );
    assert_eq!(calls.borrow().start, 0);
}
#[test]
fn invalid_end_cancels_including_missing_and_malformed_fields() {
    for message in [
        json!({}),
        json!({"ptm":"cA==","tk":"!"}),
        json!({"ptm":5,"tk":"dA=="}),
        json!({"ptm":"cA==","tk":"dA==","X-Apple-I-MD-RINFO":0}),
        json!({"ptm":"cA==","tk":"dA==","X-Apple-I-MD-RINFO":"-1"}),
    ] {
        let calls = Rc::default();
        let mut s = Session::default();
        begin(&mut s, &calls, "");
        start(&mut s, &calls);
        assert_eq!(
            send(&mut s, message, &calls, "")["result"],
            "EndProvisioningError"
        );
        drop(s);
        let c = calls.borrow();
        assert_eq!((c.finish, c.cancel, c.drop), (0, 1, 1));
    }
}

#[test]
fn routing_extension_is_optional_but_strict_when_present() {
    let base = json!({"ptm":"cA==","tk":"dA=="});
    assert_eq!(
        serde_json::from_value::<EndMessage>(base.clone())
            .unwrap()
            .routing,
        None
    );
    for value in [
        json!(1),
        json!(u64::MAX),
        json!("123456789"),
        json!("18446744073709551615"),
    ] {
        let mut message = base.clone();
        message["X-Apple-I-MD-RINFO"] = value;
        assert!(
            serde_json::from_value::<EndMessage>(message)
                .unwrap()
                .routing
                .unwrap()
                > 0
        );
    }
    for value in [
        json!(0),
        json!(-1),
        json!(null),
        json!(1.5),
        json!("+1"),
        json!(" 1"),
        json!("18446744073709551616"),
        json!(""),
    ] {
        let mut message = base.clone();
        message["X-Apple-I-MD-RINFO"] = value;
        assert!(serde_json::from_value::<EndMessage>(message).is_err());
    }
}
#[test]
fn backend_errors_are_redacted_and_cleanup_is_exactly_once() {
    for failure in ["start", "finish", "output"] {
        let calls = Rc::default();
        let mut s = Session::default();
        begin(&mut s, &calls, failure);
        let mut reply = start(&mut s, &calls);
        if failure == "finish" {
            reply = send(&mut s, json!({"ptm":"cA==","tk":"dA=="}), &calls, "");
        }
        assert!(!reply.to_string().contains("SECRET"));
        assert!(s.is_closed());
        drop(s);
        let c = calls.borrow();
        assert_eq!(c.cancel, usize::from(failure != "start"));
        assert_eq!(c.drop, 1);
    }
}
#[test]
fn oversized_message_terminates_pending_session() {
    let calls = Rc::default();
    let mut s = Session::default();
    begin(&mut s, &calls, "");
    start(&mut s, &calls);
    let reply = s
        .receive(
            &vec![b' '; MAX_MESSAGE_BYTES + 1],
            |_| -> Result<Fake, ()> { panic!() },
        )
        .unwrap();
    assert_eq!(reply["result"], "EndProvisioningError");
    assert_eq!(calls.borrow().cancel, 1);
}
