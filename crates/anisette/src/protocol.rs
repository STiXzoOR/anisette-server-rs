use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt};

pub const DSID: i64 = -2;
pub const ADI_ERR_NOT_PROVISIONED: i32 = -45061;
pub const OP_IS_PROVISIONED: u32 = 0xb0eda7af;
pub const OP_START_PROVISIONING: u32 = 0x716bd86c;
pub const OP_FINISH_PROVISIONING: u32 = 0x0b2b3196;
pub const OP_CANCEL_PROVISIONING: u32 = 0x55a637c0;
pub const OP_REQUEST_OTP: u32 = 0xcfe0b46a;
pub const OP_SET_ROUTING: u32 = 0x632b8d6e;
pub const OP_GET_ROUTING: u32 = 0x85fe63b0;
pub const OP_FREE: u32 = 0x3e58e7f9;
pub const MAX_STATE_BYTES: usize = 3 * 1024 * 1024;
/// Allows a maximum Base64 state plus 4 KiB for JSON and protocol fields.
pub const MAX_MESSAGE_BYTES: usize = MAX_STATE_BYTES.div_ceil(3) * 4 + 4096;

/// Verifies that `bytes` match an expected lowercase hex SHA-256 digest.
#[must_use]
pub fn verify_sha256(bytes: &[u8], expected_hex: &str) -> bool {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
        == expected_hex
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateSnapshot {
    pub provider: String,
    pub version: u32,
    pub binary_sha256: String,
    pub identifier: [u8; 16],
    pub host_uuid: [u8; 16],
    pub files: BTreeMap<String, String>,
}

impl StateSnapshot {
    /// Serializes a backend state envelope and enforces [`MAX_STATE_BYTES`].
    ///
    /// # Errors
    /// Returns an error if serialization fails or the output exceeds [`MAX_STATE_BYTES`].
    pub fn encode(
        provider: &str,
        binary_sha256: &str,
        identifier: [u8; 16],
        host_uuid: [u8; 16],
        files: BTreeMap<String, String>,
    ) -> Result<Vec<u8>, &'static str> {
        let state = Self {
            provider: provider.into(),
            version: 1,
            binary_sha256: binary_sha256.into(),
            identifier,
            host_uuid,
            files,
        };
        let bytes = serde_json::to_vec(&state).map_err(|_| "state serialization failed")?;
        if bytes.len() > MAX_STATE_BYTES {
            return Err("state too large");
        }
        Ok(bytes)
    }

    /// Parses and validates a backend state envelope against the expected provider, binary hash, and identity.
    ///
    /// # Errors
    /// Returns an error if the snapshot exceeds [`MAX_STATE_BYTES`], is malformed JSON, or mismatches metadata.
    pub fn decode(
        bytes: &[u8],
        provider: &str,
        binary_sha256: &str,
        identifier: [u8; 16],
    ) -> Result<Self, &'static str> {
        if bytes.len() > MAX_STATE_BYTES {
            return Err("state too large");
        }
        let state: Self = serde_json::from_slice(bytes).map_err(|_| "invalid state snapshot")?;
        if state.provider != provider
            || state.version != 1
            || state.binary_sha256 != binary_sha256
            || state.identifier != identifier
        {
            return Err("state backend, version, binary or identity mismatch");
        }
        Ok(state)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum InputError {
    TooLarge,
    InvalidJson,
    InvalidBase64,
    InvalidIdentifier,
    EmptyData,
}

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "input exceeds size limit",
            Self::InvalidJson => "invalid JSON message",
            Self::InvalidBase64 => "invalid standard Base64",
            Self::InvalidIdentifier => "identifier must contain exactly 16 bytes",
            Self::EmptyData => "binary field must not be empty",
        })
    }
}
impl std::error::Error for InputError {}

/// Error messages deliberately exclude request contents and decoder diagnostics.
///
/// # Errors
/// Returns an error if the message exceeds the wire limit or cannot deserialize into `T`.
pub fn parse_message<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, InputError> {
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(InputError::TooLarge);
    }
    serde_json::from_slice(bytes).map_err(|_| InputError::InvalidJson)
}

/// Decodes a nonempty standard Base64 field within its binary size limit.
///
/// # Errors
/// Returns an error for invalid Base64, empty data, or encoded or decoded data exceeding the limit.
pub fn decode_binary(encoded: &str, max_bytes: usize) -> Result<Vec<u8>, InputError> {
    if encoded.len() > max_bytes.div_ceil(3).saturating_mul(4) {
        return Err(InputError::TooLarge);
    }
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| InputError::InvalidBase64)?;
    if bytes.len() > max_bytes {
        return Err(InputError::TooLarge);
    }
    if bytes.is_empty() {
        return Err(InputError::EmptyData);
    }
    Ok(bytes)
}

#[derive(Clone, PartialEq, Eq)]
pub struct Identifier([u8; 16]);

impl fmt::Debug for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Identifier([redacted])")
    }
}

impl Identifier {
    #[must_use]
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Parses the canonical hyphenated UUID representation without changing byte order.
    ///
    /// # Errors
    /// Returns an error unless the input contains 32 ASCII hex digits in the canonical UUID layout.
    pub fn from_uuid(value: &str) -> Result<Self, InputError> {
        if value.len() != 36 || !value.is_ascii() {
            return Err(InputError::InvalidIdentifier);
        }
        let mut bytes = [0; 16];
        let mut source = value.bytes();
        for (i, byte) in bytes.iter_mut().enumerate() {
            if [4, 6, 8, 10].contains(&i) && source.next() != Some(b'-') {
                return Err(InputError::InvalidIdentifier);
            }
            let hi = source
                .next()
                .and_then(|c| (c as char).to_digit(16))
                .ok_or(InputError::InvalidIdentifier)?;
            let lo = source
                .next()
                .and_then(|c| (c as char).to_digit(16))
                .ok_or(InputError::InvalidIdentifier)?;
            *byte = ((hi << 4) | lo) as u8;
        }
        Ok(Self(bytes))
    }

    #[must_use]
    pub fn bytes(&self) -> [u8; 16] {
        self.0
    }
    /// Decodes an identifier from its standard Base64 representation.
    ///
    /// # Errors
    /// Returns an error unless standard Base64 decodes to exactly 16 bytes.
    pub fn decode(encoded: &str) -> Result<Self, InputError> {
        let bytes = decode_binary(encoded, 16).map_err(|_| InputError::InvalidIdentifier)?;
        Ok(Self(
            bytes
                .try_into()
                .map_err(|_| InputError::InvalidIdentifier)?,
        ))
    }

    #[must_use]
    pub fn uuid(&self) -> String {
        let b = self.0;
        format!(
            "{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
            b[0],
            b[1],
            b[2],
            b[3],
            b[4],
            b[5],
            b[6],
            b[7],
            b[8],
            b[9],
            b[10],
            b[11],
            b[12],
            b[13],
            b[14],
            b[15]
        )
    }

    /// Dadoum's Android mapping; not a mapping for macOS or Windows identities.
    #[must_use]
    pub fn android_adi_identifier(&self) -> String {
        self.uuid()[..16].to_owned()
    }
}

#[derive(Debug, Deserialize)]
pub struct IdentifierMessage {
    pub identifier: String,
}
#[derive(Debug, Deserialize)]
pub struct StartMessage {
    pub spim: String,
}
#[derive(Debug, Deserialize)]
pub struct EndMessage {
    pub ptm: String,
    pub tk: String,
    /// Optional Apple routing override (required on backends without a static V3 fallback, currently macOS); standard V3 clients omit this field.
    #[serde(default, rename = "X-Apple-I-MD-RINFO", deserialize_with = "routing")]
    pub routing: Option<u64>,
}

fn routing<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Number {
        Integer(u64),
        Decimal(String),
    }
    let value = match Number::deserialize(d)? {
        Number::Integer(n) => n,
        Number::Decimal(s) if !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()) => {
            s.parse().map_err(serde::de::Error::custom)?
        }
        _ => return Err(serde::de::Error::custom("invalid routing")),
    };
    if value == 0 {
        return Err(serde::de::Error::custom("zero routing"));
    }
    Ok(Some(value))
}
#[derive(Debug, Deserialize)]
pub struct HeadersRequest {
    pub identifier: String,
    pub adi_pb: String,
}

impl HeadersRequest {
    /// Decodes the identifier and opaque state from a header request.
    ///
    /// # Errors
    /// Returns an error for malformed Base64, an identifier other than 16 bytes, or empty or oversized state.
    pub fn decode(&self) -> Result<(Identifier, Vec<u8>), InputError> {
        Ok((
            Identifier::decode(&self.identifier)?,
            decode_binary(&self.adi_pb, MAX_STATE_BYTES)?,
        ))
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ClientInfo<'a> {
    pub client_info: &'a str,
    pub user_agent: &'a str,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(tag = "result")]
pub enum ServerMessage<'a> {
    GiveIdentifier,
    GiveStartProvisioningData,
    GiveEndProvisioningData {
        cpim: &'a str,
    },
    ProvisioningSuccess {
        adi_pb: &'a str,
    },
    InvalidIdentifier,
    Timeout,
    StartProvisioningError {
        message: &'a str,
    },
    EndProvisioningError {
        message: &'a str,
    },
    GetHeadersError {
        message: &'a str,
    },
    Headers {
        #[serde(rename = "X-Apple-I-MD")]
        otp: &'a str,
        #[serde(rename = "X-Apple-I-MD-M")]
        machine_id: &'a str,
        #[serde(rename = "X-Apple-I-MD-RINFO")]
        routing: &'a str,
    },
}

/// V1 names preserve the exact JSON casing consumed by existing clients.
/// Identity/profile values must come from the actual backend, never the host OS.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct V1Headers<'a> {
    #[serde(rename = "X-Apple-I-MD")]
    pub otp: &'a str,
    #[serde(rename = "X-Apple-I-MD-M")]
    pub machine_id: &'a str,
    #[serde(rename = "X-Apple-I-MD-RINFO")]
    pub routing: &'a str,
    #[serde(rename = "X-Apple-I-MD-LU")]
    pub local_user_id: &'a str,
    #[serde(rename = "X-Apple-I-SRL-NO")]
    pub serial: &'a str,
    #[serde(rename = "X-Mme-Device-Id")]
    pub device_id: &'a str,
    #[serde(rename = "X-Apple-I-Client-Time")]
    pub client_time: &'a str,
    #[serde(rename = "X-Apple-I-TimeZone")]
    pub timezone: &'a str,
    #[serde(rename = "X-Apple-Locale")]
    pub locale: &'a str,
    #[serde(rename = "X-MMe-Client-Info")]
    pub client_info: &'a str,
    #[serde(rename = "User-Agent")]
    pub user_agent: &'a str,
}
