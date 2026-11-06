// SPDX-License-Identifier: GPL-2.0-only
// Wire operations adapted from os-findmy-e macOS-ADI-emulator/adi.py.
use crate::{Error, Runtime};
use anisette::protocol::{
    ADI_ERR_NOT_PROVISIONED, OP_CANCEL_PROVISIONING, OP_FINISH_PROVISIONING, OP_GET_ROUTING,
    OP_IS_PROVISIONED, OP_REQUEST_OTP, OP_SET_ROUTING, OP_START_PROVISIONING,
};

fn u32_be(input: &mut &[u8]) -> Result<u32, Error> {
    let bytes = take(input, 4)?;
    Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}
fn vector(input: &mut &[u8]) -> Result<Vec<u8>, Error> {
    let size = u32_be(input)? as usize;
    Ok(take(input, size)?.to_vec())
}
fn take<'a>(input: &mut &'a [u8], n: usize) -> Result<&'a [u8], Error> {
    let bytes = input
        .get(..n)
        .ok_or(Error::Execution("truncated ADI response"))?;
    *input = &input[n..];
    Ok(bytes)
}
fn push_vector(output: &mut Vec<u8>, input: &[u8]) -> Result<(), Error> {
    if input.is_empty() || input.len() > 64 * 1024 {
        return Err(Error::Execution("invalid ADI input size"));
    }
    output.extend((input.len() as u32).to_be_bytes());
    output.extend(input);
    Ok(())
}
impl Runtime {
    fn must(&mut self, code: u32, body: &[u8]) -> Result<Vec<u8>, Error> {
        let (status, bytes) = self.call(code, body)?;
        if status != 0 {
            return Err(Error::Adi(status));
        }
        Ok(bytes)
    }
    /// Checks whether the selected device has completed provisioning.
    ///
    /// # Errors
    /// Returns an error if ADI reports an unexpected status or emulation fails.
    pub fn is_machine_provisioned(&mut self, dsid: i64) -> Result<bool, Error> {
        match self.call(OP_IS_PROVISIONED, &dsid.to_be_bytes())?.0 {
            0 => Ok(true),
            ADI_ERR_NOT_PROVISIONED => Ok(false),
            status => Err(Error::Adi(status)),
        }
    }
    /// Starts native provisioning and returns CPIM with its pending session handle.
    ///
    /// # Errors
    /// Returns an error for empty or oversized SPIM, rejected ADI requests, invalid replies, or failed emulation.
    pub fn start_provisioning(&mut self, dsid: i64, spim: &[u8]) -> Result<(Vec<u8>, u32), Error> {
        let mut body = dsid.to_be_bytes().to_vec();
        push_vector(&mut body, spim)?;
        let bytes = self.must(OP_START_PROVISIONING, &body)?;
        let mut input = bytes.as_slice();
        let cpim = vector(&mut input)?;
        let session = u32_be(&mut input)?;
        if cpim.is_empty() {
            return Err(Error::Execution("empty CPIM"));
        }
        Ok((cpim, session))
    }
    /// Completes the pending session using Apple provisioning data.
    ///
    /// # Errors
    /// Returns an error for empty or oversized data, rejected ADI requests, or failed emulation.
    pub fn finish_provisioning(
        &mut self,
        session: u32,
        ptm: &[u8],
        tk: &[u8],
    ) -> Result<(), Error> {
        let mut body = session.to_be_bytes().to_vec();
        push_vector(&mut body, ptm)?;
        push_vector(&mut body, tk)?;
        self.must(OP_FINISH_PROVISIONING, &body)?;
        Ok(())
    }
    /// Cancels a pending native provisioning session.
    ///
    /// # Errors
    /// Returns an error if ADI rejects the handle or emulation fails.
    pub fn cancel_provisioning(&mut self, session: u32) -> Result<(), Error> {
        self.must(OP_CANCEL_PROVISIONING, &session.to_be_bytes())?;
        Ok(())
    }
    /// Stores a nonzero routing value in the native backend.
    ///
    /// # Errors
    /// Returns an error for zero routing, a rejected ADI request, or failed emulation.
    pub fn set_routing(&mut self, dsid: i64, routing: u64) -> Result<(), Error> {
        if routing == 0 {
            return Err(Error::Execution("routing must not be zero"));
        }
        let mut body = dsid.to_be_bytes().to_vec();
        body.extend(routing.to_be_bytes());
        self.must(OP_SET_ROUTING, &body)?;
        Ok(())
    }
    /// Reads the routing value stored by the native backend.
    ///
    /// # Errors
    /// Returns an error if the dispatcher is unavailable, ADI rejects the request, the reply is invalid, or emulation fails.
    pub fn routing(&mut self, dsid: i64) -> Result<u64, Error> {
        let bytes = self.must(OP_GET_ROUTING, &dsid.to_be_bytes())?;
        Ok(u64::from_be_bytes(bytes.as_slice().try_into().map_err(
            |_| Error::Execution("invalid routing response"),
        )?))
    }
    /// Returns raw machine identifier and OTP. Callers must not log these bytes.
    ///
    /// # Errors
    /// Returns an error if ADI rejects the request, the reply is invalid, or emulation fails.
    pub fn request_otp(&mut self, dsid: i64) -> Result<(Vec<u8>, Vec<u8>), Error> {
        let bytes = self.must(OP_REQUEST_OTP, &dsid.to_be_bytes())?;
        let mut input = bytes.as_slice();
        let mid = vector(&mut input)?;
        let otp = vector(&mut input)?;
        if mid.is_empty() || otp.is_empty() || !input.is_empty() {
            return Err(Error::Execution("invalid OTP response"));
        }
        Ok((mid, otp))
    }
}
#[cfg(test)]
mod tests {
    use super::{push_vector, vector};
    #[test]
    fn binary_vectors_reject_truncation_and_oversize() {
        for b in [&b""[..], &b"\0\0\0\x05a"[..], &b"\xff\xff\xff\xff"[..]] {
            assert!(vector(&mut &b[..]).is_err());
        }
        let mut v = vec![];
        push_vector(&mut v, b"hello").unwrap();
        assert_eq!(vector(&mut v.as_slice()).unwrap(), b"hello");
        assert!(push_vector(&mut vec![], &vec![0; 65537]).is_err());
    }
}
