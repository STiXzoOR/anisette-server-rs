use anisette::protocol::{
    ClientInfo, HeadersRequest, Identifier, InputError, MAX_MESSAGE_BYTES, MAX_STATE_BYTES,
    ServerMessage, StartMessage, V1Headers, decode_binary, parse_message,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

#[test]
fn identifier_uses_uuid_byte_order_and_android_string_prefix() {
    let id = Identifier::decode("ABEiM0RVZneImaq7zN3u/w==").unwrap();
    assert_eq!(id.uuid(), "00112233-4455-6677-8899-AABBCCDDEEFF");
    assert_eq!(id.android_adi_identifier(), "00112233-4455-66");
}

#[test]
fn rejects_non_uuid_lengths_and_non_standard_base64() {
    for length in [0, 1, 15, 17, 32] {
        assert!(Identifier::decode(&STANDARD.encode(vec![0; length])).is_err());
    }
    for input in [
        "!",
        "ABEiM0RVZneImaq7zN3u_w==",
        "ABEiM0RVZneImaq7zN3u/w",
        " ABEiM0RVZneImaq7zN3u/w==",
    ] {
        assert!(Identifier::decode(input).is_err());
    }
}

#[test]
fn binary_size_limits_apply_before_and_after_decode() {
    assert_eq!(decode_binary("YWJj", 3).unwrap(), b"abc");
    assert_eq!(decode_binary("YWJj", 2), Err(InputError::TooLarge));
    assert_eq!(decode_binary("YWJjZA==", 3), Err(InputError::TooLarge));
    assert_eq!(decode_binary("", 3), Err(InputError::EmptyData));
    assert_eq!(decode_binary("????", 3), Err(InputError::InvalidBase64));
}

#[test]
fn json_rejects_wrong_fields_types_duplicates_and_oversize() {
    for input in [
        r#"{}"#,
        r#"{"spim":5}"#,
        r#"{"spim":"YQ==","spim":"Yg=="}"#,
        r#"{"tk":"YQ=="}"#,
    ] {
        assert!(parse_message::<StartMessage>(input.as_bytes()).is_err());
    }
    assert!(matches!(
        parse_message::<Value>(&vec![b' '; MAX_MESSAGE_BYTES + 1]),
        Err(InputError::TooLarge)
    ));
    assert!(parse_message::<StartMessage>(br#"{"spim":"YQ==","extra":true}"#).is_ok());
}

#[test]
fn maximum_state_roundtrips_through_provisioning_and_headers() {
    let state = vec![0xab; MAX_STATE_BYTES];
    let encoded = STANDARD.encode(&state);
    let response =
        serde_json::to_vec(&ServerMessage::ProvisioningSuccess { adi_pb: &encoded }).unwrap();
    let response: Value = parse_message(&response).unwrap();
    let request = serde_json::to_vec(&json!({
        "identifier": STANDARD.encode([1; 16]),
        "adi_pb": response["adi_pb"],
    }))
    .unwrap();
    let request: HeadersRequest = parse_message(&request).unwrap();
    let (identifier, decoded) = request.decode().unwrap();
    assert_eq!(identifier.bytes(), [1; 16]);
    assert_eq!(
        decoded, state,
        "maximum state must survive the wire roundtrip"
    );
}

#[test]
fn server_messages_match_wire_contract() {
    let cases = [
        (
            ServerMessage::GiveIdentifier,
            json!({"result":"GiveIdentifier"}),
        ),
        (
            ServerMessage::GiveStartProvisioningData,
            json!({"result":"GiveStartProvisioningData"}),
        ),
        (
            ServerMessage::GiveEndProvisioningData { cpim: "YQ==" },
            json!({"result":"GiveEndProvisioningData","cpim":"YQ=="}),
        ),
        (
            ServerMessage::ProvisioningSuccess { adi_pb: "Yg==" },
            json!({"result":"ProvisioningSuccess","adi_pb":"Yg=="}),
        ),
        (
            ServerMessage::InvalidIdentifier,
            json!({"result":"InvalidIdentifier"}),
        ),
        (ServerMessage::Timeout, json!({"result":"Timeout"})),
        (
            ServerMessage::StartProvisioningError { message: "failed" },
            json!({"result":"StartProvisioningError","message":"failed"}),
        ),
        (
            ServerMessage::EndProvisioningError { message: "failed" },
            json!({"result":"EndProvisioningError","message":"failed"}),
        ),
        (
            ServerMessage::GetHeadersError { message: "failed" },
            json!({"result":"GetHeadersError","message":"failed"}),
        ),
        (
            ServerMessage::Headers {
                otp: "YQ==",
                machine_id: "Yg==",
                routing: "123",
            },
            json!({"result":"Headers","X-Apple-I-MD":"YQ==","X-Apple-I-MD-M":"Yg==","X-Apple-I-MD-RINFO":"123"}),
        ),
    ];
    for (actual, expected) in cases {
        assert_eq!(serde_json::to_value(actual).unwrap(), expected);
    }
    assert_eq!(
        serde_json::to_value(ClientInfo {
            client_info: "synthetic",
            user_agent: "test"
        })
        .unwrap(),
        json!({"client_info":"synthetic","user_agent":"test"})
    );
}

#[test]
fn v1_headers_match_wire_contract() {
    let value = serde_json::to_value(V1Headers {
        otp: "YQ==",
        machine_id: "Yg==",
        routing: "123",
        local_user_id: "synthetic-user",
        serial: "0",
        device_id: "synthetic-device",
        client_time: "2026-10-30T00:00:00Z",
        timezone: "UTC",
        locale: "en_US",
        client_info: "backend-profile",
        user_agent: "backend-agent",
    })
    .unwrap();
    let expected = json!({
        "X-Apple-I-MD":"YQ==", "X-Apple-I-MD-M":"Yg==", "X-Apple-I-MD-RINFO":"123",
        "X-Apple-I-MD-LU":"synthetic-user", "X-Apple-I-SRL-NO":"0", "X-Mme-Device-Id":"synthetic-device",
        "X-Apple-I-Client-Time":"2026-10-30T00:00:00Z", "X-Apple-I-TimeZone":"UTC", "X-Apple-Locale":"en_US",
        "X-MMe-Client-Info":"backend-profile", "User-Agent":"backend-agent"
    });
    assert_eq!(value, expected);
    assert!(value.get("result").is_none());
}

#[test]
fn canonical_uuid_roundtrip_rejects_malformed_separators_and_unicode() {
    use anisette::protocol::Identifier;
    let id = Identifier::from_bytes([0xab; 16]);
    assert!(Identifier::from_uuid(&id.uuid()).unwrap() == id);
    assert!(Identifier::from_uuid(&id.uuid().to_ascii_lowercase()).unwrap() == id);
    for invalid in [
        "",
        "abababababab-abab-abab-abab-abababab",
        "ZZZZZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZZZZZZZZZ",
        "éééééééé-éééé-éééé-éééé-éééééééééééé",
    ] {
        assert!(Identifier::from_uuid(invalid).is_err());
    }
}
