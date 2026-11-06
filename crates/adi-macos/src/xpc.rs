// SPDX-License-Identifier: GPL-2.0-only
// Daemon XPC/CF contract adapted from os-findmy-e macOS-ADI-emulator/adi.py and stubs.py.
use crate::{
    Error,
    runtime::{Cpu, argument as arg, read_u64},
    stubs::cstr,
};
use std::collections::BTreeMap;
#[derive(Clone)]
pub(crate) enum Object {
    Data(Vec<u8>),
    Listener,
    Peer,
    Dictionary(BTreeMap<String, Vec<u8>>),
    String(String),
    Url(String),
}
pub(crate) fn object(cpu: &mut Cpu<'_>, value: Object) -> Result<u64, Error> {
    let p = cpu.get_data_mut().object(64)?;
    cpu.get_data_mut().objects.insert(p, value);
    Ok(p)
}
fn cell(cpu: &Cpu<'_>, name: &str) -> Result<u64, Error> {
    cpu.get_data()
        .cells
        .get(name)
        .copied()
        .ok_or(Error::Execution("missing data import"))
}
fn value(cpu: &Cpu<'_>, name: &str) -> Result<u64, Error> {
    read_u64(cpu, cell(cpu, name)?)
}
fn dictionary<'a>(cpu: &'a Cpu<'_>, p: u64) -> Result<&'a BTreeMap<String, Vec<u8>>, Error> {
    match cpu.get_data().objects.get(&p) {
        Some(Object::Dictionary(d)) => Ok(d),
        _ => Err(Error::Execution("invalid XPC dictionary")),
    }
}
pub(crate) fn call(cpu: &mut Cpu<'_>, name: &str) -> Result<Option<u64>, Error> {
    let a = arg(cpu, 0)?;
    let b = arg(cpu, 1)?;
    let c = arg(cpu, 2)?;
    let d = arg(cpu, 3)?;
    let result = match name {
        "xpc_connection_create_mach_service" => object(cpu, Object::Listener)?,
        "xpc_connection_set_event_handler" => {
            let bytes = cpu.mem_read_as_vec(b, 48)?;
            let p = cpu.get_data_mut().object(48)?;
            cpu.mem_write(p, &bytes)?;
            match cpu.get_data().objects.get(&a) {
                Some(Object::Listener) => cpu.get_data_mut().listener_block = Some(p),
                Some(Object::Peer) => cpu.get_data_mut().peer_block = Some(p),
                _ => return Err(Error::Execution("unknown XPC connection")),
            }
            0
        }
        "xpc_get_type" => cell(
            cpu,
            if matches!(cpu.get_data().objects.get(&a), Some(Object::Dictionary(_))) {
                "xpc_type_dictionary"
            } else {
                "xpc_type_error"
            },
        )?,
        "xpc_dictionary_create_reply" => object(cpu, Object::Dictionary(BTreeMap::new()))?,
        "xpc_dictionary_get_data" => {
            let key = cstr(cpu, b)?;
            let data = dictionary(cpu, a)?.get(&key).cloned();
            match data {
                Some(bytes) => {
                    let p = cpu.get_data_mut().scratch(bytes.len() as u64 + 8)?;
                    cpu.mem_write(p, &bytes)?;
                    if c != 0 {
                        cpu.mem_write(c, &(bytes.len() as u64).to_le_bytes())?;
                    }
                    p
                }
                None => 0,
            }
        }
        "xpc_dictionary_set_data" => {
            if d > 4 * 1024 * 1024 {
                return Err(Error::Execution("XPC data too large"));
            }
            let key = cstr(cpu, b)?;
            let data = cpu.mem_read_as_vec(c, d as usize)?;
            match cpu.get_data_mut().objects.get_mut(&a) {
                Some(Object::Dictionary(map)) => {
                    if map.len() >= 8 && !map.contains_key(&key) {
                        return Err(Error::Execution("too many XPC fields"));
                    }
                    map.insert(key, data);
                }
                _ => return Err(Error::Execution("invalid reply object")),
            }
            0
        }
        "xpc_dictionary_send_reply" => {
            cpu.get_data_mut().reply = Some(dictionary(cpu, a)?.clone());
            0
        }
        "xpc_dictionary_get_value" | "xpc_dictionary_get_uint64" | "xpc_dictionary_get_string" => 0,
        "xpc_dictionary_get_audit_token" => {
            if b != 0 {
                let mut bytes = Vec::new();
                for v in [501u32, 501, 0, 501, 0, 1234, 0, 0] {
                    bytes.extend(v.to_le_bytes());
                }
                cpu.mem_write(b, &bytes)?;
            }
            0
        }
        "audit_token_to_euid" => 501,
        "audit_token_to_au32" => {
            for (i, v) in [501u32, 501, 0, 501, 0, 1234].iter().enumerate() {
                let p = arg(cpu, i)?;
                if p != 0 {
                    cpu.mem_write(p, &v.to_le_bytes())?;
                }
            }
            0
        }
        "CFDataCreateWithBytesNoCopy" => {
            if c > 4 * 1024 * 1024 {
                return Err(Error::Execution("CFData too large"));
            }
            let bytes = cpu.mem_read_as_vec(b, c as usize)?;
            object(cpu, Object::Data(bytes))?
        }
        "CFPropertyListCreateWithData" => {
            // The synthetic Setup Assistant marker is empty, hence not a valid property list.
            match cpu.get_data().objects.get(&b) {
                Some(Object::Data(bytes)) if bytes.is_empty() => 0,
                _ => return Err(Error::Execution("nonempty property list unsupported")),
            }
        }
        "CFDataGetLength" => match cpu.get_data().objects.get(&a) {
            Some(Object::Data(bytes)) => bytes.len() as u64,
            _ => return Err(Error::Execution("invalid CFData")),
        },
        "CFDataGetBytePtr" => {
            let Some(Object::Data(bytes)) = cpu.get_data().objects.get(&a) else {
                return Err(Error::Execution("invalid CFData"));
            };
            let bytes = bytes.clone();
            let p = cpu.get_data_mut().scratch(bytes.len() as u64)?;
            cpu.mem_write(p, &bytes)?;
            p
        }
        "CFStringCreateWithCStringNoCopy" => {
            let s = cstr(cpu, b)?;
            object(cpu, Object::String(s))?
        }
        "CFNumberGetTypeID" => 22,
        "CFStringGetTypeID" => 7,
        "CFDictionaryGetTypeID" => 18,
        "CFGetTypeID" => {
            if matches!(cpu.get_data().objects.get(&a), Some(Object::String(_))) {
                7
            } else {
                99
            }
        }
        "CFStringGetCString" => {
            let Some(Object::String(s)) = cpu.get_data().objects.get(&a) else {
                return Err(Error::Execution("invalid CFString"));
            };
            let mut bytes = s.as_bytes().to_vec();
            bytes.push(0);
            if bytes.len() as u64 > c {
                return Ok(Some(0));
            }
            cpu.mem_write(b, &bytes)?;
            1
        }
        "SecTaskCreateWithAuditToken" => cpu.get_data_mut().object(32)?,
        "SecTaskCopyValueForEntitlement" => {
            if matches!(
                cpu.get_data().objects.get(&b),
                Some(Object::String(s)) if s == "adi-client" || s == "com.apple.private.adid"
            ) {
                object(cpu, Object::String("1".into()))?
            } else {
                value(cpu, "kCFBooleanTrue")?
            }
        }
        "proc_pidpath" => {
            let path = b"/System/Library/PrivateFrameworks/AuthKit.framework/Support/akd\0";
            if c < path.len() as u64 {
                return Err(Error::Execution("pid path buffer too short"));
            }
            cpu.mem_write(b, path)?;
            (path.len() - 1) as u64
        }
        "strtoul" => {
            let s = cstr(cpu, a)?;
            let base = if c == 0 {
                10
            } else {
                u32::try_from(c).unwrap_or(0)
            };
            if !(2..=36).contains(&base) {
                return Err(Error::Execution("invalid strtoul base"));
            }
            let mut n = 0u64;
            let mut len = 0;
            for ch in s.chars() {
                let Some(digit) = ch.to_digit(base) else {
                    break;
                };
                n = n
                    .saturating_mul(u64::from(base))
                    .saturating_add(u64::from(digit));
                len += ch.len_utf8() as u64;
            }
            if b != 0 {
                cpu.mem_write(b, &(a + len).to_le_bytes())?;
            }
            n
        }
        "CFURLCreateFromFileSystemRepresentation" => {
            if c > 4096 {
                return Err(Error::Execution("URL too long"));
            }
            let path = String::from_utf8(cpu.mem_read_as_vec(b, c as usize)?)
                .map_err(|_| Error::Execution("invalid URL"))?;
            if !crate::filesystem::FileSystem::valid_path(&path) {
                return Err(Error::Execution("invalid URL path"));
            }
            object(cpu, Object::Url(path))?
        }
        "CFURLCopyResourcePropertyForKey" => {
            let Some(Object::Url(path)) = cpu.get_data().objects.get(&a) else {
                return Err(Error::Execution("invalid CFURL"));
            };
            let is_dir = cpu.get_data().fs.dirs.contains(path);
            let key =
                cpu.get_data().cells.iter().find_map(|(name, _)| {
                    (value(cpu, name).ok() == Some(b)).then_some(name.clone())
                });
            let output = match key.as_deref() {
                Some("kCFURLIsDirectoryKey") => value(
                    cpu,
                    if is_dir {
                        "kCFBooleanTrue"
                    } else {
                        "kCFBooleanFalse"
                    },
                )?,
                Some("kCFURLIsHiddenKey") => value(cpu, "kCFBooleanFalse")?,
                Some("kCFURLFileSecurityKey" | "kCFURLTypeIdentifierKey") => {
                    cpu.get_data_mut().object(32)?
                }
                _ => return Ok(Some(0)),
            };
            cpu.mem_write(c, &output.to_le_bytes())?;
            1
        }
        "objc_msgSend" => match cstr(cpu, b)?.as_str() {
            "processInfo" => cpu.get_data_mut().object(256)?,
            "operatingSystem" => 0x1a,
            "isOperatingSystemAtLeastVersion:" => 1,
            _ => return Err(Error::Execution("unsupported Objective-C selector")),
        },
        _ => return Ok(None),
    };
    Ok(Some(result))
}
