//! Apple device provisioning over HTTPS. Never accepts Apple account credentials.
use crate::protocol::decode_binary;
use base64::{Engine, engine::general_purpose::STANDARD};
use plist::{Dictionary, Value};
use reqwest::{
    Certificate,
    blocking::{Client, RequestBuilder},
    header::{HeaderMap, HeaderName, HeaderValue},
    redirect,
};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    io::{Cursor, Read},
    sync::LazyLock,
    time::Duration,
};
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
const MAX_FIELD_BYTES: usize = 65536;
static APPLE_ROOT_CA: LazyLock<Certificate> = LazyLock::new(|| {
    Certificate::from_pem(include_bytes!("../apple-root-ca.pem")).expect("valid Apple Root CA PEM")
});
fn field<'a>(v: &'a Value, key: &str) -> Result<&'a Value> {
    v.as_dictionary()
        .and_then(|d| d.get(key))
        .ok_or("missing plist field".into())
}
fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    field(v, key)?
        .as_string()
        .ok_or("expected plist string".into())
}
const MAX_RESPONSE: usize = 1024 * 1024;
fn response(request: RequestBuilder, stage: &str) -> Result<Value> {
    let response = request.send().map_err(|error| {
        let kind = if error.is_timeout() {
            "timed out"
        } else if error.is_connect() {
            "connection failed (DNS, proxy or TLS)"
        } else {
            "request failed"
        };
        format!("Apple {stage} {kind}: {}", error.without_url())
    })?;
    if !response.status().is_success() {
        return Err(format!("Apple {stage} returned HTTP {}", response.status()).into());
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_RESPONSE as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Apple {stage} response read failed: {error}"))?;
    if bytes.len() > MAX_RESPONSE {
        return Err(format!("Apple {stage} response too large").into());
    }
    Value::from_reader(Cursor::new(bytes))
        .map_err(|error| format!("Apple {stage} invalid plist: {error}").into())
}
fn endpoint(value: &Value, key: &str) -> Result<url::Url> {
    let url = url::Url::parse(string(value, key)?)?;
    if url.scheme() != "https"
        || url.host_str() != Some("gsa.apple.com")
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("unexpected Apple provisioning endpoint".into());
    }
    Ok(url)
}
fn post(client: &Client, url: url::Url, request: Dictionary, stage: &str) -> Result<Value> {
    let root = Value::Dictionary(Dictionary::from_iter([
        ("Header", Value::Dictionary(Dictionary::new())),
        ("Request", Value::Dictionary(request)),
    ]));
    let mut bytes = vec![];
    root.to_writer_xml(&mut bytes)?;
    let value = response(
        client
            .post(url)
            .header("X-Apple-I-Client-Time", client_time()?)
            .body(bytes),
        stage,
    )?;
    let response = field(&value, "Response")?;
    let status = field(response, "Status")?;
    if field(status, "ec")?.as_signed_integer() != Some(0) {
        return Err(format!("Apple {stage} device provisioning rejected").into());
    }
    Ok(response.clone())
}
#[must_use]
pub fn local_user_id(identifier: [u8; 16]) -> String {
    Sha256::digest(identifier)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect()
}
/// Formats the current UTC time for Apple request headers.
///
/// # Errors
/// Returns an error if the timestamp cannot be normalized or formatted as RFC 3339.
pub fn client_time() -> Result<String> {
    Ok(time::OffsetDateTime::now_utc()
        .replace_nanosecond(0)?
        .format(&time::format_description::well_known::Rfc3339)?)
}
/// One Apple device exchange, also usable by a V3 client. No account credentials.
pub struct Exchange {
    client: Client,
    finish_url: url::Url,
}

impl fmt::Debug for Exchange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Exchange")
            .field("finish_url", &self.finish_url)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct FinishData {
    pub ptm: Vec<u8>,
    pub tk: Vec<u8>,
    pub routing: u64,
}
impl Exchange {
    /// Contacts Apple to obtain start data for one device provisioning exchange.
    ///
    /// # Errors
    /// Returns an error for invalid profile headers, transport failures, untrusted endpoints, or invalid Apple responses.
    pub fn start(
        identifier: [u8; 16],
        profile: crate::protocol::ClientInfo<'_>,
    ) -> Result<(Self, Vec<u8>)> {
        let device = crate::protocol::Identifier::from_bytes(identifier).uuid();
        let lu = local_user_id(identifier);
        let mut headers = HeaderMap::new();
        for (name, value) in [
            ("X-Mme-Device-Id", device.as_str()),
            ("X-Apple-I-MD-LU", lu.as_str()),
            ("X-Apple-I-SRL-NO", "0"),
            ("X-MMe-Client-Info", profile.client_info),
            ("User-Agent", profile.user_agent),
            ("X-Apple-Client-App-Name", "Setup"),
            ("Content-Type", "application/x-www-form-urlencoded"),
        ] {
            if value.len() > 512 || !value.bytes().all(|b| (32..127).contains(&b)) {
                return Err("invalid provisioning header".into());
            }
            let mut value = HeaderValue::from_str(value)?;
            value.set_sensitive(true);
            headers.insert(HeaderName::from_bytes(name.as_bytes())?, value);
        }
        // Apple's provisioning root is absent from common Linux trust stores.
        let client = Client::builder()
            .tls_certs_merge([APPLE_ROOT_CA.clone()])
            .default_headers(headers)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .redirect(redirect::Policy::none())
            .http1_only()
            .build()?;
        let bag = response(
            client.get("https://gsa.apple.com/grandslam/GsService2/lookup"),
            "lookup",
        )?;
        let urls = field(&bag, "urls")?;
        let start = post(
            &client,
            endpoint(urls, "midStartProvisioning")?,
            Dictionary::new(),
            "start provisioning",
        )?;
        let spim = decode_binary(string(&start, "spim")?, MAX_FIELD_BYTES)?;
        Ok((
            Self {
                client,
                finish_url: endpoint(urls, "midFinishProvisioning")?,
            },
            spim,
        ))
    }
    /// Submits CPIM to Apple and returns the completion data and routing value.
    ///
    /// # Errors
    /// Returns an error for empty or oversized CPIM, transfer failures, rejected provisioning, or invalid completion fields.
    pub fn finish(self, cpim: &[u8]) -> Result<FinishData> {
        if cpim.is_empty() || cpim.len() > MAX_FIELD_BYTES {
            return Err("invalid CPIM size".into());
        }
        let finish = post(
            &self.client,
            self.finish_url,
            Dictionary::from_iter([("cpim", Value::String(STANDARD.encode(cpim)))]),
            "finish provisioning",
        )?;
        let value = field(&finish, "X-Apple-I-MD-RINFO")?;
        let routing = value
            .as_unsigned_integer()
            .or_else(|| value.as_string()?.parse().ok())
            .filter(|n| *n != 0)
            .ok_or("invalid routing")?;
        Ok(FinishData {
            ptm: decode_binary(string(&finish, "ptm")?, MAX_FIELD_BYTES)?,
            tk: decode_binary(string(&finish, "tk")?, MAX_FIELD_BYTES)?,
            routing,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Dictionary, Value, endpoint};
    #[test]
    fn lookup_cannot_redirect_provisioning_to_untrusted_endpoints() {
        for url in [
            "http://gsa.apple.com/x",
            "https://example.com/x",
            "https://gsa.apple.com.evil.test/x",
            "https://user@gsa.apple.com/x",
            "https://gsa.apple.com:444/x",
            "file:///etc/passwd",
        ] {
            let value =
                Value::Dictionary(Dictionary::from_iter([("url", Value::String(url.into()))]));
            assert!(endpoint(&value, "url").is_err());
        }
        let value = Value::Dictionary(Dictionary::from_iter([(
            "url",
            Value::String("https://gsa.apple.com/x".into()),
        )]));
        assert!(endpoint(&value, "url").is_ok());
    }
}
