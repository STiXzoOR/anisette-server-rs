// ARM64 call and import behavior derived from Anisette.py 627c9c6, MIT.
// Copyright (c) 2025 Anisette.py Contributors; see LICENSE-MIT-Anisette.py.
use crate::{Error, Libraries};
use anisette::{
    memory::Heap,
    protocol::{
        ADI_ERR_NOT_PROVISIONED, DSID, Identifier, MAX_STATE_BYTES, OP_GET_ROUTING, OP_SET_ROUTING,
    },
};
use std::{collections::BTreeMap, fmt};
use unicorn_engine::{
    RegisterARM64 as Reg, Unicorn,
    unicorn_const::{Arch, Mode, Prot},
};

const HEAP: u64 = 0x60000000;
const HEAP_SIZE: u64 = 0x1000000;
const SCRATCH: u64 = 0x80000000;
const SCRATCH_SIZE: u64 = 0x100000;
const ERRNO: u64 = SCRATCH + SCRATCH_SIZE - 16;
const IMPORT: u64 = 0xa0000000;
const IMPORT_SIZE: u64 = 0x10000;
const RETURN: u64 = 0xdead0000;
const STACK: u64 = 0xf0000000;
const STACK_SIZE: u64 = 0x100000;
const BASES: [u64; 2] = [0x00100000, 0x10100000];
const REGS: [Reg; 8] = [
    Reg::X0,
    Reg::X1,
    Reg::X2,
    Reg::X3,
    Reg::X4,
    Reg::X5,
    Reg::X6,
    Reg::X7,
];

#[derive(Default)]
struct StateFile {
    bytes: Option<Vec<u8>>,
    position: usize,
    opened: bool,
}
impl StateFile {
    fn open(&mut self, flags: u64) -> Result<u64, i32> {
        if self.opened {
            return Err(16);
        }
        if self.bytes.is_none() {
            if flags & 0x40 == 0 {
                return Err(2);
            }
            self.bytes = Some(vec![]);
        }
        if flags & 0x200 != 0 {
            self.bytes = Some(vec![]);
        }
        self.position = 0;
        self.opened = true;
        Ok(69)
    }
    fn check(&self, fd: u64) -> Result<(), i32> {
        if fd != 69 || !self.opened {
            Err(9)
        } else {
            Ok(())
        }
    }
    fn truncate(&mut self, fd: u64, size: usize) -> Result<(), i32> {
        self.check(fd)?;
        if size > MAX_STATE_BYTES {
            return Err(27);
        }
        self.bytes.as_mut().ok_or(2)?.resize(size, 0);
        Ok(())
    }
    fn read(&mut self, fd: u64, size: usize) -> Result<Vec<u8>, i32> {
        self.check(fd)?;
        let bytes = self.bytes.as_ref().ok_or(2)?;
        let start = self.position.min(bytes.len());
        let end = start.saturating_add(size).min(bytes.len());
        self.position = end;
        Ok(bytes[start..end].to_vec())
    }
    fn write(&mut self, fd: u64, data: &[u8]) -> Result<usize, i32> {
        self.check(fd)?;
        let end = self
            .position
            .checked_add(data.len())
            .filter(|n| *n <= MAX_STATE_BYTES)
            .ok_or(27)?;
        let bytes = self.bytes.as_mut().ok_or(2)?;
        bytes.resize(bytes.len().max(end), 0);
        bytes[self.position..end].copy_from_slice(data);
        self.position = end;
        Ok(data.len())
    }
}
struct Env {
    heap: Heap,
    imports: BTreeMap<u64, String>,
    exports: [BTreeMap<String, u64>; 2],
    file: StateFile,
    fault: Option<Error>,
}
type Cpu<'a> = Unicorn<'a, Env>;
pub struct Runtime {
    cpu: Cpu<'static>,
    scratch: u64,
    poisoned: bool,
}
impl fmt::Debug for Runtime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Runtime")
            .field("poisoned", &self.poisoned)
            .finish_non_exhaustive()
    }
}
impl Runtime {
    /// Creates an Android emulator with an identity and optional raw state.
    ///
    /// # Errors
    /// Returns an error for invalid state size, memory mapping or relocation failures, or failed native initialization.
    pub fn new(
        libraries: &Libraries,
        identifier: Identifier,
        state: Option<&[u8]>,
    ) -> Result<Self, Error> {
        if state.is_some_and(|s| s.is_empty() || s.len() > MAX_STATE_BYTES) {
            return Err("invalid Android state size".into());
        }
        let env = Env {
            heap: Heap::new(HEAP, HEAP_SIZE)?,
            imports: BTreeMap::new(),
            exports: Default::default(),
            file: StateFile {
                bytes: state.map(<[u8]>::to_vec),
                ..Default::default()
            },
            fault: None,
        };
        let mut cpu = Unicorn::new_with_data(Arch::ARM64, Mode::LITTLE_ENDIAN, env)?;
        for (address, size, prot) in [
            (HEAP, HEAP_SIZE, Prot::READ | Prot::WRITE),
            (SCRATCH, SCRATCH_SIZE, Prot::READ | Prot::WRITE),
            (RETURN, 4096, Prot::READ | Prot::EXEC),
            (STACK, STACK_SIZE, Prot::READ | Prot::WRITE),
        ] {
            cpu.mem_map(address, size, prot)?;
        }
        for (index, image) in libraries.images.iter().enumerate() {
            let base = BASES[index];
            let imports = IMPORT + index as u64 * 0x1000000;
            cpu.mem_map(base, image.size, Prot::ALL)?;
            cpu.mem_map(imports, IMPORT_SIZE, Prot::READ | Prot::WRITE)?;
            cpu.mem_write(
                imports,
                &[0xc0, 0x03, 0x5f, 0xd6].repeat(IMPORT_SIZE as usize / 4),
            )?;
            cpu.mem_protect(imports, IMPORT_SIZE, Prot::READ | Prot::EXEC)?;
            for (offset, range) in &image.segments {
                cpu.mem_write(base + offset, &image.bytes[range.clone()])?;
            }
            let mut symbols = vec![];
            for (i, (name, address)) in image.symbols.iter().enumerate() {
                let value = match address {
                    Some(offset) => base + offset,
                    None if name.is_empty() => 0,
                    None => {
                        let address = imports + i as u64 * 4;
                        cpu.get_data_mut().imports.insert(address, name.clone());
                        address
                    }
                };
                symbols.push(value);
            }
            for reloc in &image.relocations {
                let value = if reloc.r_type == 1027 {
                    base
                } else {
                    symbols[reloc.r_sym]
                }
                .wrapping_add_signed(reloc.r_addend.unwrap_or(0));
                cpu.mem_write(base + reloc.r_offset, &value.to_le_bytes())?;
            }
            cpu.get_data_mut().exports[index] = image
                .exports
                .iter()
                .map(|(s, a)| (s.clone(), base + a))
                .collect();
            cpu.add_code_hook(imports, imports + IMPORT_SIZE - 1, |cpu, address, _| {
                let result = (|| {
                    let name = cpu
                        .get_data()
                        .imports
                        .get(&address)
                        .ok_or("unknown import address")?
                        .clone();
                    import(cpu, &name)
                })();
                match result {
                    Ok(value) => {
                        if let Err(e) = cpu.reg_write(Reg::X0, value) {
                            cpu.get_data_mut().fault = Some(e.into());
                            let _ = cpu.emu_stop();
                        }
                    }
                    Err(e) => {
                        cpu.get_data_mut().fault = Some(e);
                        let _ = cpu.emu_stop();
                    }
                }
            })?;
        }
        let mut runtime = Self {
            cpu,
            scratch: SCRATCH,
            poisoned: false,
        };
        let id = identifier.android_adi_identifier();
        let ptr = runtime.data(id.as_bytes())?;
        runtime.must("Sph98paBcz", &[ptr, id.len() as u64])?;
        let path = runtime.data(b".\0")?;
        runtime.must("nf92ngaK92", &[path])?;
        runtime.must("kq56gsgHG6", &[path])?;
        Ok(runtime)
    }
    fn invoke(&mut self, address: u64, args: &[u64]) -> Result<i32, Error> {
        if self.poisoned {
            return Err("runtime unusable after failed execution".into());
        }
        if args.len() > REGS.len() {
            return Err("too many ARM64 arguments".into());
        }
        self.poisoned = true;
        for (i, reg) in REGS.iter().enumerate() {
            self.cpu
                .reg_write(*reg, args.get(i).copied().unwrap_or(0))?;
        }
        self.cpu.reg_write(Reg::SP, STACK + STACK_SIZE - 4096)?;
        self.cpu.reg_write(Reg::LR, RETURN)?;
        self.cpu.get_data_mut().fault = None;
        self.cpu.emu_start(address, RETURN, 5_000_000, 50_000_000)?;
        if let Some(error) = self.cpu.get_data_mut().fault.take() {
            return Err(error);
        }
        if self.cpu.reg_read(Reg::PC)? != RETURN {
            return Err("execution budget exhausted".into());
        }
        self.poisoned = false;
        Ok(self.cpu.reg_read(Reg::X0)? as u32 as i32)
    }
    fn call(&mut self, name: &str, args: &[u64]) -> Result<i32, Error> {
        let address = *self.cpu.get_data().exports[0]
            .get(name)
            .ok_or("missing ADI export")?;
        self.invoke(address, args)
    }
    fn must(&mut self, name: &str, args: &[u64]) -> Result<(), Error> {
        match self.call(name, args)? {
            0 => Ok(()),
            status => Err(Error::Adi(status)),
        }
    }
    fn data(&mut self, bytes: &[u8]) -> Result<u64, Error> {
        let size = (bytes.len() as u64).max(8).div_ceil(16) * 16;
        let end = self
            .scratch
            .checked_add(size)
            .filter(|n| *n < ERRNO)
            .ok_or("scratch exhausted")?;
        let ptr = self.scratch;
        self.cpu.mem_write(ptr, bytes)?;
        self.scratch = end;
        Ok(ptr)
    }
    fn output(&mut self, pointer: u64, length: u64) -> Result<(u64, Vec<u8>), Error> {
        let ptr = read_u64_le(&self.cpu, pointer)?;
        let len = read_u32_le(&self.cpu, length)? as usize;
        if ptr == 0 || len == 0 || len > MAX_STATE_BYTES {
            return Err("invalid ADI output size".into());
        }
        Ok((ptr, self.cpu.mem_read_as_vec(ptr, len)?))
    }
    /// Checks whether the selected device has completed provisioning.
    ///
    /// # Errors
    /// Returns an error if ADI reports an unexpected status or emulation fails.
    pub fn is_machine_provisioned(&mut self) -> Result<bool, Error> {
        match self.call("aslgmuibau", &[DSID as u64])? {
            0 => Ok(true),
            ADI_ERR_NOT_PROVISIONED => Ok(false),
            n => Err(Error::Adi(n)),
        }
    }
    /// Starts native provisioning and returns CPIM with its pending session handle.
    ///
    /// # Errors
    /// Returns an error for empty or oversized SPIM, rejected ADI requests, invalid replies, or failed emulation.
    pub fn start_provisioning(&mut self, spim: &[u8]) -> Result<(Vec<u8>, u32), Error> {
        if spim.is_empty() || spim.len() > 65536 {
            return Err("invalid SPIM size".into());
        }
        self.scratch = SCRATCH;
        let cpim = self.data(&[0; 8])?;
        let length = self.data(&[0; 4])?;
        let session = self.data(&[0; 4])?;
        let input = self.data(spim)?;
        self.must(
            "rsegvyrt87",
            &[DSID as u64, input, spim.len() as u64, cpim, length, session],
        )?;
        let handle = read_u32_le(&self.cpu, session)?;
        let result = (|| {
            let (ptr, bytes) = self.output(cpim, length)?;
            self.must("jk24uiwqrg", &[ptr])?;
            Ok((bytes, handle))
        })();
        if result.is_err() {
            let _ = self.cancel_provisioning(handle);
        }
        result
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
        if ptm.is_empty() || tk.is_empty() || ptm.len() > 65536 || tk.len() > 65536 {
            return Err("invalid finish data size".into());
        }
        self.scratch = SCRATCH;
        let p = self.data(ptm)?;
        let t = self.data(tk)?;
        self.must(
            "uv5t6nhkui",
            &[session as u64, p, ptm.len() as u64, t, tk.len() as u64],
        )
    }
    /// Cancels a pending native provisioning session.
    ///
    /// # Errors
    /// Returns an error if ADI rejects the handle or emulation fails.
    pub fn cancel_provisioning(&mut self, session: u32) -> Result<(), Error> {
        self.must("fy34trz2st", &[session as u64])
    }
    /// Returns the machine identifier and a fresh OTP without logging either.
    ///
    /// # Errors
    /// Returns an error if ADI rejects the request, the reply is invalid, or emulation fails.
    pub fn request_otp(&mut self) -> Result<(Vec<u8>, Vec<u8>), Error> {
        self.scratch = SCRATCH;
        let mid = self.data(&[0; 8])?;
        let ml = self.data(&[0; 4])?;
        let otp = self.data(&[0; 8])?;
        let ol = self.data(&[0; 4])?;
        self.must("qi864985u0", &[DSID as u64, mid, ml, otp, ol])?;
        let (mp, m) = self.output(mid, ml)?;
        let (op, o) = self.output(otp, ol)?;
        self.must("jk24uiwqrg", &[mp])?;
        self.must("jk24uiwqrg", &[op])?;
        Ok((m, o))
    }
    /// Copies the raw Android state for later restoration.
    ///
    /// # Errors
    /// Returns an error if the runtime is poisoned or has no nonempty state.
    pub fn export_state(&self) -> Result<Vec<u8>, Error> {
        if self.poisoned {
            return Err("runtime unusable after failed execution".into());
        }
        self.cpu
            .get_data()
            .file
            .bytes
            .clone()
            .filter(|b| !b.is_empty())
            .ok_or("no Android state".into())
    }

    // Dispatcher framing also documented by os-findmy-e native/adi-core (GPL-2.0-only).
    fn dispatch(&mut self, code: u32, body: &[u8]) -> Result<(), Error> {
        let mut request = vec![0; 2048];
        if body.len() > request.len() - 8 {
            return Err("dispatcher body too large".into());
        }
        request[3] = 1;
        request[7] = 1;
        request[8..8 + body.len()].copy_from_slice(body);
        let buffer = self.data(&request)?;
        let mut params = [0u8; 32];
        params[..8].copy_from_slice(&buffer.to_le_bytes());
        params[8..12].copy_from_slice(&(8 + body.len() as u32).to_le_bytes());
        params[12..16].copy_from_slice(&4u32.to_le_bytes());
        let params = self.data(&params)?;
        let address = *self.cpu.get_data().exports[1]
            .get("vdfut768ig")
            .ok_or("missing CoreADI dispatcher")?;
        let rc = self.invoke(address, &[code as u64, params])?;
        if rc != 0 {
            return Err(Error::Adi(rc));
        }
        let status = read_i32_be(&self.cpu, buffer)?;
        if status != 0 {
            return Err(Error::Adi(status));
        }
        Ok(())
    }
    /// Reads the routing value stored by the native backend.
    ///
    /// # Errors
    /// Returns an error if the dispatcher is unavailable, ADI rejects the request, the reply is invalid, or emulation fails.
    pub fn routing(&mut self) -> Result<u64, Error> {
        self.scratch = SCRATCH;
        let output = self.data(&[0; 8])?;
        let mut body = DSID.to_be_bytes().to_vec();
        body.extend(output.to_be_bytes());
        self.dispatch(OP_GET_ROUTING, &body)?;
        read_u64_le(&self.cpu, output)
    }
    /// Stores a nonzero routing value in the native backend.
    ///
    /// # Errors
    /// Returns an error for zero routing, a rejected ADI request, or failed emulation.
    pub fn set_routing(&mut self, routing: u64) -> Result<(), Error> {
        if routing == 0 {
            return Err("routing must not be zero".into());
        }
        self.scratch = SCRATCH;
        let mut body = DSID.to_be_bytes().to_vec();
        body.extend(routing.to_be_bytes());
        self.dispatch(OP_SET_ROUTING, &body)
    }
}
fn read_u32_le(cpu: &Cpu<'_>, address: u64) -> Result<u32, Error> {
    let mut bytes = [0u8; 4];
    cpu.mem_read(address, &mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}
fn read_i32_be(cpu: &Cpu<'_>, address: u64) -> Result<i32, Error> {
    let mut bytes = [0u8; 4];
    cpu.mem_read(address, &mut bytes)?;
    Ok(i32::from_be_bytes(bytes))
}
fn read_u64_le(cpu: &Cpu<'_>, address: u64) -> Result<u64, Error> {
    let mut bytes = [0u8; 8];
    cpu.mem_read(address, &mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}
fn arg(cpu: &Cpu<'_>, i: usize) -> Result<u64, Error> {
    Ok(cpu.reg_read(REGS[i])?)
}
fn text(cpu: &Cpu<'_>, ptr: u64) -> Result<String, Error> {
    let mut bytes = Vec::new();
    while bytes.len() < 4096 {
        let addr = ptr
            .checked_add(bytes.len() as u64)
            .ok_or("string overflow")?;
        let chunk_len = 64usize
            .min(4096 - ((addr as usize) & 0xfff))
            .min(4096 - bytes.len());
        let chunk = cpu.mem_read_as_vec(addr, chunk_len)?;
        if let Some(pos) = chunk.iter().position(|&b| b == 0) {
            bytes.extend_from_slice(&chunk[..pos]);
            return String::from_utf8(bytes).map_err(|_| "invalid guest string".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Err("unterminated guest string".into())
}
fn size(n: u64) -> Result<usize, Error> {
    if n > HEAP_SIZE {
        Err("guest buffer too large".into())
    } else {
        Ok(n as usize)
    }
}
fn io(cpu: &mut Cpu<'_>, result: Result<u64, i32>) -> Result<u64, Error> {
    match result {
        Ok(value) => Ok(value),
        Err(errno) => {
            cpu.mem_write(ERRNO, &errno.to_le_bytes())?;
            Ok(u64::MAX)
        }
    }
}
fn stat(cpu: &mut Cpu<'_>, is_dir: bool, ptr: u64) -> Result<u64, Error> {
    let length = if is_dir {
        4096
    } else {
        match &cpu.get_data().file.bytes {
            Some(b) => b.len(),
            None => return io(cpu, Err(2)),
        }
    };
    let mut bytes = [0u8; 128];
    bytes[16..20].copy_from_slice(&(if is_dir { 16877u32 } else { 33188 }).to_le_bytes());
    bytes[24..28].copy_from_slice(&33188u32.to_le_bytes());
    bytes[48..56].copy_from_slice(&(length as u64).to_le_bytes());
    // ADI requires a nonzero mtime; keep this virtual file metadata stable across restores.
    bytes[88..96].copy_from_slice(&0x01010000u64.to_le_bytes());
    cpu.mem_write(ptr, &bytes)?;
    Ok(0)
}
fn import(cpu: &mut Cpu<'_>, name: &str) -> Result<u64, Error> {
    let a = arg(cpu, 0)?;
    let b = arg(cpu, 1)?;
    let c = arg(cpu, 2)?;
    match name {
        "malloc" => Ok(cpu.get_data_mut().heap.allocate(a, 16)?),
        "free" => {
            cpu.get_data_mut().heap.release(a)?;
            Ok(0)
        }
        "strncpy" => {
            let n = size(c)?;
            let source = text(cpu, b)?;
            let mut bytes = vec![0; n];
            let len = source.len().min(n);
            bytes[..len].copy_from_slice(&source.as_bytes()[..len]);
            cpu.mem_write(a, &bytes)?;
            Ok(a)
        }
        "dlopen" => {
            let path = text(cpu, a)?;
            if !matches!(path.as_str(), "libCoreADI.so" | "./libCoreADI.so") {
                return Err("unexpected library path".into());
            }
            Ok(2)
        }
        "dlsym" => {
            if a != 2 {
                return Err("invalid library handle".into());
            }
            let name = text(cpu, b)?;
            cpu.get_data().exports[1]
                .get(&name)
                .copied()
                .ok_or("unknown CoreADI export".into())
        }
        "dlclose" => Ok(0),
        "__errno" => Ok(ERRNO),
        "__system_property_get" => {
            cpu.mem_write(b, b"no s/n number\0")?;
            Ok(13)
        }
        "arc4random" => {
            let mut value = [0; 4];
            getrandom::fill(&mut value).map_err(|_| "host randomness failed")?;
            Ok(u32::from_le_bytes(value) as u64)
        }
        "gettimeofday" => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| "clock before epoch")?;
            if a != 0 {
                cpu.mem_write(a, &now.as_secs().to_le_bytes())?;
                cpu.mem_write(a + 8, &u64::from(now.subsec_micros()).to_le_bytes())?;
            }
            if b != 0 {
                cpu.mem_write(b, &[0; 8])?;
            }
            Ok(0)
        }
        "lstat" => {
            let path = text(cpu, a)?;
            match path.as_str() {
                "." => stat(cpu, true, b),
                "./adi.pb" => stat(cpu, false, b),
                _ => Err("unexpected virtual path".into()),
            }
        }
        "fstat" => {
            if let Err(e) = cpu.get_data().file.check(a) {
                return io(cpu, Err(e));
            }
            stat(cpu, false, b)
        }
        "open" => {
            if text(cpu, a)? != "./adi.pb" {
                return Err("unexpected virtual path".into());
            }
            let r = cpu.get_data_mut().file.open(b);
            io(cpu, r)
        }
        "close" => {
            let r = cpu.get_data().file.check(a);
            if r.is_ok() {
                cpu.get_data_mut().file.opened = false;
            }
            io(cpu, r.map(|_| 0))
        }
        "ftruncate" => {
            let n = size(b)?;
            let r = cpu.get_data_mut().file.truncate(a, n);
            io(cpu, r.map(|_| 0))
        }
        "read" => {
            let n = size(c)?;
            match cpu.get_data_mut().file.read(a, n) {
                Ok(bytes) => {
                    cpu.mem_write(b, &bytes)?;
                    Ok(bytes.len() as u64)
                }
                Err(e) => io(cpu, Err(e)),
            }
        }
        "write" => {
            let bytes = cpu.mem_read_as_vec(b, size(c)?)?;
            let r = cpu.get_data_mut().file.write(a, &bytes);
            io(cpu, r.map(|n| n as u64))
        }
        "mkdir" => {
            if text(cpu, a)? != "." {
                return Err("unexpected virtual directory".into());
            }
            io(cpu, Err(17))
        }
        "chmod" => {
            if !matches!(text(cpu, a)?.as_str(), "." | "./adi.pb") {
                return Err("unexpected virtual path".into());
            }
            Ok(0)
        }
        "umask" => Ok(0o777),
        // No guest threads are started; synchronization stays within this one emulator.
        "pthread_once"
        | "pthread_rwlock_init"
        | "pthread_rwlock_destroy"
        | "pthread_rwlock_rdlock"
        | "pthread_rwlock_wrlock"
        | "pthread_rwlock_unlock" => Ok(0),
        _ => Err(Error::UnsupportedImport(name.into())),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Arch, BTreeMap, Env, Error, HEAP, HEAP_SIZE, Heap, MAX_STATE_BYTES, Mode, Prot, Runtime,
        SCRATCH, StateFile, Unicorn,
    };
    #[test]
    fn virtual_state_enforces_handles_offsets_and_size() {
        let mut file = StateFile::default();
        assert_eq!(file.open(0), Err(2));
        assert_eq!(file.open(0x42), Ok(69));
        assert_eq!(file.open(0x42), Err(16));
        assert_eq!(file.write(70, b"wrong"), Err(9));
        file.write(69, b"hello").unwrap();
        file.write(69, b" world").unwrap();
        file.position = 0;
        assert_eq!(file.read(69, 5).unwrap(), b"hello");
        assert_eq!(file.read(69, 99).unwrap(), b" world");
        assert!(file.truncate(69, MAX_STATE_BYTES + 1).is_err());
        file.truncate(69, 2).unwrap();
        assert_eq!(file.bytes.unwrap(), b"he");
    }

    #[test]
    fn arm64_loop_stops_and_poisoned_runtime_cannot_continue() {
        let env = Env {
            heap: Heap::new(HEAP, HEAP_SIZE).unwrap(),
            imports: BTreeMap::new(),
            exports: Default::default(),
            file: Default::default(),
            fault: None,
        };
        let mut cpu = Unicorn::new_with_data(Arch::ARM64, Mode::LITTLE_ENDIAN, env).unwrap();
        cpu.mem_map(0x1000, 4096, Prot::ALL).unwrap();
        cpu.mem_write(0x1000, &[0, 0, 0, 0x14]).unwrap();
        let mut runtime = Runtime {
            cpu,
            scratch: SCRATCH,
            poisoned: false,
        };
        assert!(matches!(
            runtime.invoke(0x1000, &[]),
            Err(Error::Invalid("execution budget exhausted"))
        ));
        assert!(matches!(
            runtime.invoke(0x1000, &[]),
            Err(Error::Invalid("runtime unusable after failed execution"))
        ));
    }
}
