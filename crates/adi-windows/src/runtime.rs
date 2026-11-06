mod state;
use crate::{Binary, Error};
use anisette::{
    memory::Heap,
    protocol::{
        ADI_ERR_NOT_PROVISIONED, DSID, OP_CANCEL_PROVISIONING, OP_FINISH_PROVISIONING, OP_FREE,
        OP_GET_ROUTING, OP_IS_PROVISIONED, OP_REQUEST_OTP, OP_SET_ROUTING, OP_START_PROVISIONING,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};
use unicorn_engine::{
    RegisterX86 as Reg, Unicorn,
    unicorn_const::{Arch, Mode, Prot},
};
const HEAP: u64 = 0x40000000;
const HEAP_SIZE: u64 = 0x1000000;
const STACK: u64 = 0x50000000;
const STACK_SIZE: u64 = 0x100000;
const SCRATCH: u64 = 0x60000000;
const IMPORT: u64 = 0x70000000;
const RETURN: u64 = 0x71000000;
const TEB: u64 = 0x10000;
const REGS: [Reg; 4] = [Reg::RCX, Reg::RDX, Reg::R8, Reg::R9];
enum SyncObject {
    Mutex(u32),
    Event { manual: bool, signaled: bool },
}
struct Env {
    fs: crate::filesystem::FileSystem,
    heap: Heap,
    allocations: BTreeMap<u64, u64>,
    imports: BTreeMap<u64, String>,
    tls: Vec<Option<u64>>,
    handles: BTreeMap<u64, SyncObject>,
    names: BTreeMap<String, u64>,
    references: BTreeMap<u64, u32>,
    directories: BTreeSet<String>,
    host_uuid: [u8; 16],
    last_error: u32,
    fault: Option<Error>,
}
type Cpu<'a> = Unicorn<'a, Env>;
pub struct Runtime {
    cpu: Cpu<'static>,
    dispatch: u64,
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
    /// Creates a Windows emulator and runs the verified DLL attach entry point.
    ///
    /// # Errors
    /// Returns an error if randomness, memory setup, import binding, or DLL attach fails.
    pub fn new(binary: &Binary) -> Result<Self, Error> {
        let image = &binary.image;
        let mut host_uuid = [0; 16];
        getrandom::fill(&mut host_uuid).map_err(|_| "host randomness unavailable")?;
        let env = Env {
            fs: crate::filesystem::FileSystem::default(),
            heap: Heap::new(HEAP, HEAP_SIZE)?,
            allocations: BTreeMap::new(),
            imports: BTreeMap::new(),
            tls: vec![],
            handles: BTreeMap::new(),
            names: BTreeMap::new(),
            references: BTreeMap::new(),
            directories: BTreeSet::from(["c:".into(), "c:\\programdata".into()]),
            host_uuid,
            last_error: 0,
            fault: None,
        };
        let mut cpu = Unicorn::new_with_data(Arch::X86, Mode::MODE_64, env)?;
        for (address, size, prot) in [
            (HEAP, HEAP_SIZE, Prot::READ | Prot::WRITE),
            (STACK, STACK_SIZE, Prot::READ | Prot::WRITE),
            (SCRATCH, 0x100000, Prot::READ | Prot::WRITE),
            (IMPORT, 0x10000, Prot::READ | Prot::WRITE),
            (RETURN, 4096, Prot::READ | Prot::EXEC),
            (TEB, 0x10000, Prot::READ | Prot::WRITE),
            (image.base, image.memory.len() as u64, Prot::ALL),
        ] {
            cpu.mem_map(address, size, prot)?;
        }
        cpu.mem_write(image.base, &image.memory)?;
        cpu.mem_write(IMPORT, &vec![0xc3; 0x10000])?;
        cpu.mem_protect(IMPORT, 0x10000, Prot::READ | Prot::EXEC)?;
        cpu.reg_write(Reg::GS_BASE, TEB)?;
        cpu.mem_write(TEB + 8, &(STACK + STACK_SIZE).to_le_bytes())?;
        cpu.mem_write(TEB + 16, &STACK.to_le_bytes())?;
        cpu.mem_write(TEB + 0x30, &TEB.to_le_bytes())?;
        for (index, (offset, name)) in image.imports.iter().enumerate() {
            let address = IMPORT + index as u64 * 16;
            cpu.get_data_mut().imports.insert(address, name.clone());
            cpu.mem_write(image.base + offset, &address.to_le_bytes())?;
        }
        cpu.add_code_hook(IMPORT, IMPORT + 0xffff, |cpu, address, _| {
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
                    if let Err(e) = cpu.reg_write(Reg::RAX, value) {
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
        let mut runtime = Self {
            cpu,
            dispatch: *image
                .exports
                .get("vdfut768ig")
                .ok_or("missing Windows dispatcher")?,
            poisoned: false,
        };
        if runtime.invoke(image.entry, &[image.base, 1, 0])? == 0 {
            return Err("Windows DLL attach failed".into());
        }
        Ok(runtime)
    }
    fn invoke(&mut self, address: u64, args: &[u64]) -> Result<u64, Error> {
        if self.poisoned {
            return Err("runtime unusable after failed execution".into());
        }
        if args.len() > 16 {
            return Err("too many Windows arguments".into());
        }
        self.poisoned = true;
        let stack = STACK + STACK_SIZE - 4096 - 8;
        self.cpu.mem_write(STACK, &vec![0; STACK_SIZE as usize])?;
        self.cpu.mem_write(stack, &RETURN.to_le_bytes())?;
        self.cpu.reg_write(Reg::RSP, stack)?;
        for (i, reg) in REGS.iter().enumerate() {
            self.cpu
                .reg_write(*reg, args.get(i).copied().unwrap_or(0))?;
        }
        for (i, value) in args.iter().enumerate().skip(4) {
            self.cpu
                .mem_write(stack + 8 + i as u64 * 8, &value.to_le_bytes())?;
        }
        self.cpu.get_data_mut().fault = None;
        self.cpu.emu_start(address, RETURN, 5_000_000, 50_000_000)?;
        if let Some(error) = self.cpu.get_data_mut().fault.take() {
            return Err(error);
        }
        if self.cpu.reg_read(Reg::RIP)? != RETURN {
            return Err("Windows execution budget exhausted".into());
        }
        self.poisoned = false;
        Ok(self.cpu.reg_read(Reg::RAX)?)
    }
    /// Initializes native ADI and returns its operation status for the caller to inspect.
    ///
    /// # Errors
    /// Returns an error if dispatch or emulation fails; negative operation statuses can also be returned in `Ok`.
    pub fn initialize(&mut self) -> Result<i32, Error> {
        self.operation(0x12db31c5, &[])
    }
    /// Checks whether the Windows device has completed provisioning.
    ///
    /// # Errors
    /// Returns an error if ADI reports an unexpected status or emulation fails.
    pub fn is_provisioned(&mut self) -> Result<bool, Error> {
        match self.operation(OP_IS_PROVISIONED, &DSID.to_be_bytes())? {
            0 => Ok(true),
            ADI_ERR_NOT_PROVISIONED => Ok(false),
            status => Err(Error::Adi(status)),
        }
    }
    fn checked(&mut self, code: u32, message: &[u8]) -> Result<(), Error> {
        let status = self.operation(code, message)?;
        if status < 0 {
            return Err(Error::Adi(status));
        }
        Ok(())
    }
    fn input(&mut self, offset: u64, bytes: &[u8]) -> Result<u64, Error> {
        if bytes.is_empty() || bytes.len() > 65536 {
            return Err("invalid ADI input size".into());
        }
        let ptr = SCRATCH + offset;
        self.cpu.mem_write(ptr, bytes)?;
        Ok(ptr)
    }
    fn output(&self, cell: u64) -> Result<(u64, Vec<u8>), Error> {
        let ptr = read_u64(&self.cpu, cell)?;
        let len = read_u32(&self.cpu, cell + 8)? as usize;
        if ptr == 0 || len == 0 || len > 65536 {
            return Err("invalid ADI output".into());
        }
        Ok((ptr, self.cpu.mem_read_as_vec(ptr, len)?))
    }
    /// Starts native provisioning and returns CPIM with its pending session handle.
    ///
    /// # Errors
    /// Returns an error for empty or oversized SPIM, rejected ADI requests, invalid replies, or failed emulation.
    pub fn start_provisioning(&mut self, spim: &[u8]) -> Result<(Vec<u8>, u32), Error> {
        let ptr = self.input(0x10000, spim)?;
        let cells = SCRATCH + 0x40000;
        self.cpu.mem_write(cells, &[0; 24])?;
        let mut body = DSID.to_be_bytes().to_vec();
        body.extend(ptr.to_be_bytes());
        body.extend((spim.len() as u32).to_be_bytes());
        for p in [cells, cells + 8, cells + 16] {
            body.extend(p.to_be_bytes());
        }
        self.checked(OP_START_PROVISIONING, &body)?;
        let session = read_u32(&self.cpu, cells + 16)?;
        let result = (|| {
            let (cpim_ptr, cpim) = self.output(cells)?;
            self.checked(OP_FREE, &cpim_ptr.to_be_bytes())?;
            Ok((cpim, session))
        })();
        if result.is_err() {
            let _ = self.cancel_provisioning(session);
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
        let ptm_ptr = self.input(0x10000, ptm)?;
        let tk_ptr = self.input(0x20000, tk)?;
        let mut body = session.to_be_bytes().to_vec();
        body.extend(ptm_ptr.to_be_bytes());
        body.extend((ptm.len() as u32).to_be_bytes());
        body.extend(tk_ptr.to_be_bytes());
        body.extend((tk.len() as u32).to_be_bytes());
        self.checked(OP_FINISH_PROVISIONING, &body)
    }
    /// Cancels a pending native provisioning session.
    ///
    /// # Errors
    /// Returns an error if ADI rejects the handle or emulation fails.
    pub fn cancel_provisioning(&mut self, session: u32) -> Result<(), Error> {
        self.checked(OP_CANCEL_PROVISIONING, &session.to_be_bytes())
    }
    /// Returns the machine identifier and a fresh OTP without logging either.
    ///
    /// # Errors
    /// Returns an error if ADI rejects the request, the reply is invalid, or emulation fails.
    pub fn request_otp(&mut self) -> Result<(Vec<u8>, Vec<u8>), Error> {
        let cells = SCRATCH + 0x40000;
        self.cpu.mem_write(cells, &[0; 32])?;
        let mut body = DSID.to_be_bytes().to_vec();
        for p in [cells, cells + 8, cells + 16, cells + 24] {
            body.extend(p.to_be_bytes());
        }
        self.checked(OP_REQUEST_OTP, &body)?;
        let (mid_ptr, mid) = self.output(cells)?;
        let (otp_ptr, otp) = self.output(cells + 16)?;
        self.checked(OP_FREE, &mid_ptr.to_be_bytes())?;
        self.checked(OP_FREE, &otp_ptr.to_be_bytes())?;
        Ok((mid, otp))
    }
    /// Stores a nonzero routing value in the native backend.
    ///
    /// # Errors
    /// Returns an error for zero routing, a rejected ADI request, or failed emulation.
    pub fn set_routing(&mut self, routing: u64) -> Result<(), Error> {
        if routing == 0 {
            return Err("routing must not be zero".into());
        }
        let mut body = DSID.to_be_bytes().to_vec();
        body.extend(routing.to_be_bytes());
        self.checked(OP_SET_ROUTING, &body)
    }
    /// Reads the routing value stored by the native backend.
    ///
    /// # Errors
    /// Returns an error if the dispatcher is unavailable, ADI rejects the request, the reply is invalid, or emulation fails.
    pub fn routing(&mut self) -> Result<u64, Error> {
        let cell = SCRATCH + 0x40000;
        self.cpu.mem_write(cell, &[0; 8])?;
        let mut body = DSID.to_be_bytes().to_vec();
        body.extend(cell.to_be_bytes());
        self.checked(OP_GET_ROUTING, &body)?;
        read_u64(&self.cpu, cell)
    }
    fn operation(&mut self, code: u32, message: &[u8]) -> Result<i32, Error> {
        if message.len() > 1024 {
            return Err("ADI frame too large".into());
        }
        // GPL-2.0-only framing reference: os-findmy-e native/adi-core/src/adi.rs.
        self.cpu.mem_write(SCRATCH, &[0; 2048])?;
        self.cpu.mem_write(SCRATCH + 3, &[1, 0, 0, 0, 1])?;
        self.cpu.mem_write(SCRATCH + 8, message)?;
        let mut params = [0u8; 32];
        params[..8].copy_from_slice(&SCRATCH.to_le_bytes());
        params[8..12].copy_from_slice(&(8u32 + message.len() as u32).to_le_bytes());
        params[12..16].copy_from_slice(&4u32.to_le_bytes());
        self.cpu.mem_write(SCRATCH + 4096, &params)?;
        let status = self.invoke(self.dispatch, &[code as u64, SCRATCH + 4096])? as u32 as i32;
        if status < 0 {
            return Err(Error::Adi(status));
        }
        read_i32_be(&self.cpu, SCRATCH)
    }
}
fn read_u16(cpu: &Cpu<'_>, address: u64) -> Result<u16, Error> {
    let mut bytes = [0u8; 2];
    cpu.mem_read(address, &mut bytes)?;
    Ok(u16::from_le_bytes(bytes))
}
fn read_u32(cpu: &Cpu<'_>, address: u64) -> Result<u32, Error> {
    let mut bytes = [0u8; 4];
    cpu.mem_read(address, &mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}
fn read_i32_be(cpu: &Cpu<'_>, address: u64) -> Result<i32, Error> {
    let mut bytes = [0u8; 4];
    cpu.mem_read(address, &mut bytes)?;
    Ok(i32::from_be_bytes(bytes))
}
fn read_u64(cpu: &Cpu<'_>, address: u64) -> Result<u64, Error> {
    let mut bytes = [0u8; 8];
    cpu.mem_read(address, &mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}
fn arg(cpu: &Cpu<'_>, index: usize) -> Result<u64, Error> {
    if index < 4 {
        Ok(cpu.reg_read(REGS[index])?)
    } else {
        read_u64(cpu, cpu.reg_read(Reg::RSP)? + 8 + index as u64 * 8)
    }
}
fn wide(cpu: &Cpu<'_>, address: u64) -> Result<String, Error> {
    if address == 0 {
        return Ok(String::new());
    }
    let mut chars = vec![];
    for i in 0..512 {
        let c = read_u16(cpu, address + i * 2)?;
        if c == 0 {
            return String::from_utf16(&chars).map_err(|_| "invalid Windows string".into());
        }
        chars.push(c);
    }
    Err("Windows string too long".into())
}
fn ansi(cpu: &Cpu<'_>, address: u64) -> Result<String, Error> {
    let mut bytes = vec![];
    for i in 0..1024 {
        let c = cpu.mem_read_as_vec(address + i, 1)?[0];
        if c == 0 {
            return Ok(bytes.iter().map(|&b| cp1252(b)).collect());
        }
        bytes.push(c);
    }
    Err("Windows string too long".into())
}
fn allocate(cpu: &mut Cpu<'_>, size: u64, zero: bool) -> Result<u64, Error> {
    if size > HEAP_SIZE {
        return Err("Windows allocation too large".into());
    }
    let ptr = cpu.get_data_mut().heap.allocate(size, 16)?;
    cpu.get_data_mut().allocations.insert(ptr, size);
    if zero {
        cpu.mem_write(ptr, &vec![0; size as usize])?;
    }
    Ok(ptr)
}
fn import(cpu: &mut Cpu<'_>, name: &str) -> Result<u64, Error> {
    let a = arg(cpu, 0)?;
    let b = arg(cpu, 1)?;
    let c = arg(cpu, 2)?;
    let d = arg(cpu, 3)?;
    match name {
        "CreateMutexA" | "CreateEventA" => {
            let ptr = if name == "CreateMutexA" { c } else { d };
            let object_name = if ptr == 0 {
                String::new()
            } else {
                ansi(cpu, ptr)?
            };
            if let Some(&handle) = cpu.get_data().names.get(&object_name) {
                let is_mutex = matches!(
                    cpu.get_data().handles.get(&handle),
                    Some(SyncObject::Mutex(_))
                );
                if is_mutex != (name == "CreateMutexA") {
                    return Err("named handle type mismatch".into());
                }
                let refs = cpu
                    .get_data_mut()
                    .references
                    .get_mut(&handle)
                    .ok_or("missing handle refs")?;
                *refs = refs
                    .checked_add(1)
                    .filter(|n| *n <= 1024)
                    .ok_or("handle reference limit")?;
                cpu.get_data_mut().last_error = 183;
                return Ok(handle);
            }
            let handle = (0x8000..0x8040)
                .find(|h| !cpu.get_data().handles.contains_key(h))
                .ok_or("Windows handle limit")?;
            let object = if name == "CreateMutexA" {
                SyncObject::Mutex(u32::from(b != 0))
            } else {
                SyncObject::Event {
                    manual: b != 0,
                    signaled: c != 0,
                }
            };
            cpu.get_data_mut().handles.insert(handle, object);
            cpu.get_data_mut().references.insert(handle, 1);
            if !object_name.is_empty() {
                cpu.get_data_mut().names.insert(object_name, handle);
            }
            cpu.get_data_mut().last_error = 0;
            Ok(handle)
        }
        "CloseHandle" => {
            if cpu.get_data_mut().fs.close(a) {
                return Ok(1);
            }
            let refs = cpu
                .get_data_mut()
                .references
                .get_mut(&a)
                .ok_or("invalid Windows handle")?;
            *refs -= 1;
            if *refs == 0 {
                let data = cpu.get_data_mut();
                data.references.remove(&a);
                data.handles.remove(&a);
                data.names.retain(|_, handle| *handle != a);
            }
            Ok(1)
        }
        "WaitForSingleObject" => {
            match cpu
                .get_data_mut()
                .handles
                .get_mut(&a)
                .ok_or("invalid Windows handle")?
            {
                SyncObject::Mutex(depth) => {
                    *depth = depth.checked_add(1).ok_or("mutex depth overflow")?;
                    Ok(0)
                }
                SyncObject::Event { manual, signaled } if *signaled => {
                    if !*manual {
                        *signaled = false;
                    }
                    Ok(0)
                }
                _ if b as u32 != u32::MAX => Ok(258),
                _ => Err("guest wait requires another thread".into()),
            }
        }
        "ReleaseMutex" => match cpu.get_data_mut().handles.get_mut(&a) {
            Some(SyncObject::Mutex(depth)) if *depth > 0 => {
                *depth -= 1;
                Ok(1)
            }
            _ => Err("invalid mutex release".into()),
        },
        "SetEvent" | "ResetEvent" => match cpu.get_data_mut().handles.get_mut(&a) {
            Some(SyncObject::Event { signaled, .. }) => {
                *signaled = name == "SetEvent";
                Ok(1)
            }
            _ => Err("invalid event".into()),
        },
        "CryptAcquireContextA" => {
            if arg(cpu, 4)? as u32 != 0xf0000000 {
                return Err("persistent crypto keysets unsupported".into());
            }
            cpu.mem_write(a, &0xcafeu64.to_le_bytes())?;
            Ok(1)
        }
        "CryptReleaseContext" => {
            if a != 0xcafe {
                return Err("invalid crypto handle".into());
            }
            Ok(1)
        }
        "CryptGenRandom" => {
            if a != 0xcafe || b > 1024 * 1024 {
                return Err("invalid random request".into());
            }
            let mut bytes = vec![0; b as usize];
            getrandom::fill(&mut bytes).map_err(|_| "host randomness unavailable")?;
            cpu.mem_write(c, &bytes)?;
            Ok(1)
        }
        "GetUserNameA" => {
            let name = b"anisette\0";
            let capacity = read_u32(cpu, b)?;
            cpu.mem_write(b, &(name.len() as u32).to_le_bytes())?;
            if capacity < name.len() as u32 {
                cpu.get_data_mut().last_error = 122;
                return Ok(0);
            }
            cpu.mem_write(a, name)?;
            Ok(1)
        }
        "RegOpenKeyExA" => {
            let key = ansi(cpu, b)?.to_ascii_lowercase();
            if a as u32 != 0x80000002 {
                return Err("unsupported registry hive".into());
            }
            let handle: u64 = match key.as_str() {
                "hardware\\description\\system" => 0xa000,
                "hardware\\description\\system\\centralprocessor\\0" => 0xa001,
                _ => return Err("unsupported registry key".into()),
            };
            cpu.mem_write(arg(cpu, 4)?, &handle.to_le_bytes())?;
            Ok(0)
        }
        "RegQueryValueExA" => {
            if a != 0xa000 && a != 0xa001 {
                return Err("invalid registry handle".into());
            }
            let value = ansi(cpu, b)?.to_ascii_lowercase();
            let (kind, bytes): (u32, &[u8]) = match (a, value.as_str()) {
                (0xa000, "systembiosversion") => (7, b"Anisette Virtual BIOS 1.0\0\0"),
                (0xa000, "videobiosversion") => (7, b"Anisette Virtual Display 1.0\0\0"),
                (0xa000, "systembiosdate") => (1, b"10/30/26\0"),
                (0xa001, "processornamestring") => (1, b"Intel(R) Core(TM) i7 CPU\0"),
                (0xa001, "vendoridentifier") => (1, b"GenuineIntel\0"),
                (0xa001, "identifier") => (1, b"Intel64 Family 6 Model 158 Stepping 10\0"),
                _ => return Ok(2),
            };
            let out = arg(cpu, 4)?;
            let size = arg(cpu, 5)?;
            if d != 0 {
                cpu.mem_write(d, &kind.to_le_bytes())?;
            }
            if size == 0 {
                if out != 0 {
                    return Err("registry size missing".into());
                }
                return Ok(0);
            }
            let capacity = read_u32(cpu, size)?;
            cpu.mem_write(size, &(bytes.len() as u32).to_le_bytes())?;
            if out != 0 {
                if capacity < bytes.len() as u32 {
                    return Ok(234);
                }
                cpu.mem_write(out, bytes)?;
            }
            Ok(0)
        }
        "RegCloseKey" if a == 0xa000 || a == 0xa001 => Ok(0),
        "GetVolumeInformationW" => {
            if a != 0 && !wide(cpu, a)?.eq_ignore_ascii_case("C:\\") {
                return Err("unsupported volume".into());
            }
            for (ptr, capacity, value) in [
                (b, c as u32, "Anisette"),
                (arg(cpu, 6)?, arg(cpu, 7)? as u32, "NTFS"),
            ] {
                if ptr != 0 {
                    if capacity <= value.len() as u32 {
                        cpu.get_data_mut().last_error = 122;
                        return Ok(0);
                    }
                    write_wide(cpu, ptr, value)?;
                }
            }
            let uuid = cpu.get_data().host_uuid;
            let serial = u32::from_le_bytes([uuid[0], uuid[1], uuid[2], uuid[3]]);
            for (ptr, value) in [(d, serial), (arg(cpu, 4)?, 255), (arg(cpu, 5)?, 6)] {
                if ptr != 0 {
                    cpu.mem_write(ptr, &value.to_le_bytes())?;
                }
            }
            Ok(1)
        }
        "CreateFileW" => {
            let path = wide(cpu, a)?.to_ascii_lowercase();
            let creation = arg(cpu, 4)? as u32;
            let flags = arg(cpu, 5)? as u32;
            if flags & 0x40000000 != 0 {
                return Err("asynchronous files unsupported".into());
            }
            let exists = cpu.get_data().fs.files.contains_key(&path);
            match cpu
                .get_data_mut()
                .fs
                .open(path, b as u32, c as u32, creation)
            {
                Ok(handle) => {
                    cpu.get_data_mut().last_error = if exists && (creation == 2 || creation == 4) {
                        183
                    } else {
                        0
                    };
                    Ok(handle)
                }
                Err(error) => {
                    cpu.get_data_mut().last_error = error;
                    Ok(u64::MAX)
                }
            }
        }
        "ReadFile" | "WriteFile" => {
            if arg(cpu, 4)? != 0 || c as u32 > 1024 * 1024 {
                return Err("unsupported file IO".into());
            }
            let count = if name == "ReadFile" {
                let bytes = cpu.get_data_mut().fs.read(a, c as u32 as usize)?;
                cpu.mem_write(b, &bytes)?;
                bytes.len()
            } else {
                let bytes = cpu.mem_read_as_vec(b, c as u32 as usize)?;
                cpu.get_data_mut().fs.write(a, &bytes)?;
                bytes.len()
            };
            if d != 0 {
                cpu.mem_write(d, &(count as u32).to_le_bytes())?;
            }
            Ok(1)
        }
        "GetFileType" if cpu.get_data().fs.contains(a) => Ok(1),
        "FlushFileBuffers" if cpu.get_data().fs.contains(a) => Ok(1),
        "SetFilePointerEx" => {
            let offset = cpu.get_data_mut().fs.seek(a, b as i64, d as u32)?;
            if c != 0 {
                cpu.mem_write(c, &(offset as u64).to_le_bytes())?;
            }
            Ok(1)
        }
        "SetEndOfFile" => {
            cpu.get_data_mut().fs.truncate(a)?;
            Ok(1)
        }
        "DeleteFileW" => {
            let path = wide(cpu, a)?.to_ascii_lowercase();
            match cpu.get_data_mut().fs.delete(&path) {
                Ok(()) => Ok(1),
                Err(e) => {
                    cpu.get_data_mut().last_error = e;
                    Ok(0)
                }
            }
        }
        "GetFileInformationByHandle" => {
            let size = cpu.get_data().fs.size(a)? as u32;
            let mut info = [0u8; 52];
            info[..4].copy_from_slice(&0x80u32.to_le_bytes());
            info[28..32].copy_from_slice(&cpu.get_data().host_uuid[..4]);
            info[36..40].copy_from_slice(&size.to_le_bytes());
            info[40..44].copy_from_slice(&1u32.to_le_bytes());
            let file_id = cpu.get_data().fs.file_id(a)?;
            info[44..48].copy_from_slice(&((file_id >> 32) as u32).to_le_bytes());
            info[48..52].copy_from_slice(&(file_id as u32).to_le_bytes());
            cpu.mem_write(b, &info)?;
            Ok(1)
        }
        "SetFileAttributesA" | "SetFileAttributesW" => {
            let path = if name.ends_with('A') {
                ansi(cpu, a)?
            } else {
                wide(cpu, a)?
            }
            .to_ascii_lowercase();
            if b as u32 & !0x86 != 0 {
                return Err("unsupported file attributes".into());
            }
            if !cpu.get_data().fs.files.contains_key(&path) {
                cpu.get_data_mut().last_error = 2;
                return Ok(0);
            }
            // Hidden/system attributes do not change access within the private in-memory guest.
            Ok(1)
        }
        "GetCurrentDirectoryW" => {
            if a as u32 <= 3 {
                return Ok(4);
            }
            write_wide(cpu, b, "C:\\")?;
            Ok(3)
        }
        "GetFullPathNameW" => {
            let path = wide(cpu, a)?;
            if !path.to_ascii_lowercase().starts_with("c:\\programdata\\")
                || path.split('\\').any(|p| p == ".." || p == ".")
            {
                return Err("unsupported full path".into());
            }
            let count = path.encode_utf16().count() as u64;
            if b as u32 as u64 <= count {
                return Ok(count + 1);
            }
            write_wide(cpu, c, &path)?;
            if d != 0 {
                let offset = path
                    .rsplit_once('\\')
                    .map_or(0, |(parent, _)| parent.encode_utf16().count() + 1)
                    as u64;
                cpu.mem_write(d, &(c + offset * 2).to_le_bytes())?;
            }
            Ok(count)
        }
        "SHGetFolderPathW" => {
            if b & 0xff != 0x23 {
                return Err("unsupported guest folder".into());
            }
            let out = arg(cpu, 4)?;
            cpu.mem_write(out, &[0; 520])?;
            write_wide(cpu, out, "C:\\ProgramData")?;
            Ok(0)
        }
        "PathAppendW" => {
            let mut path = wide(cpu, a)?;
            let part = wide(cpu, b)?;
            if part.contains(':') || part.starts_with('\\') || part.split('\\').any(|p| p == "..") {
                return Err("invalid appended path".into());
            }
            if !path.ends_with('\\') {
                path.push('\\');
            }
            path.push_str(&part);
            write_wide(cpu, a, &path)?;
            Ok(1)
        }
        "PathIsDirectoryW" | "GetFileAttributesW" | "GetFileAttributesA" => {
            let path = if name.ends_with('A') {
                ansi(cpu, a)?
            } else {
                wide(cpu, a)?
            }
            .trim_end_matches('\\')
            .to_ascii_lowercase();
            if cpu.get_data().directories.contains(&path) {
                Ok(if name == "PathIsDirectoryW" { 1 } else { 16 })
            } else if cpu.get_data().fs.files.contains_key(&path) {
                Ok(if name == "PathIsDirectoryW" { 0 } else { 0x80 })
            } else {
                cpu.get_data_mut().last_error = 2;
                Ok(if name == "PathIsDirectoryW" {
                    0
                } else {
                    u32::MAX as u64
                })
            }
        }
        "CreateDirectoryW" => {
            let path = wide(cpu, a)?.trim_end_matches('\\').to_ascii_lowercase();
            if !path.starts_with("c:\\programdata\\")
                || path
                    .split('\\')
                    .any(|p| p.is_empty() || p == ".." || p == ".")
                || cpu.get_data().directories.len() >= 64
            {
                return Err("invalid virtual directory".into());
            }
            if cpu.get_data_mut().directories.insert(path) {
                Ok(1)
            } else {
                cpu.get_data_mut().last_error = 183;
                Ok(0)
            }
        }
        "GetProcessHeap" => Ok(0x42),
        "HeapAlloc" => allocate(cpu, c, b & 8 != 0),
        "HeapFree" => {
            cpu.get_data_mut().heap.release(c)?;
            cpu.get_data_mut().allocations.remove(&c);
            Ok(1)
        }
        "HeapSize" => Ok(*cpu
            .get_data()
            .allocations
            .get(&c)
            .ok_or("invalid Windows allocation")?),
        "HeapReAlloc" => {
            let old = *cpu
                .get_data()
                .allocations
                .get(&c)
                .ok_or("invalid Windows allocation")?;
            let bytes = cpu.mem_read_as_vec(c, old.min(d) as usize)?;
            let new = allocate(cpu, d, b & 8 != 0)?;
            cpu.mem_write(new, &bytes)?;
            cpu.get_data_mut().heap.release(c)?;
            cpu.get_data_mut().allocations.remove(&c);
            Ok(new)
        }
        "GetCurrentProcessId" | "GetCurrentThreadId" => Ok(1),
        "GetCurrentProcess" => Ok(u64::MAX),
        "GetLastError" => Ok(cpu.get_data().last_error as u64),
        "SetLastError" => {
            cpu.get_data_mut().last_error = a as u32;
            Ok(0)
        }
        "GetSystemTimeAsFileTime" | "QueryPerformanceCounter" => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| "invalid host clock")?;
            let value = if name == "GetSystemTimeAsFileTime" {
                116444736000000000 + now.as_nanos() as u64 / 100
            } else {
                now.as_nanos() as u64
            };
            cpu.mem_write(a, &value.to_le_bytes())?;
            Ok(1)
        }
        "InitializeSListHead" => {
            cpu.mem_write(a, &[0; 16])?;
            Ok(0)
        }
        "InitializeCriticalSectionAndSpinCount" => {
            cpu.mem_write(a, &[0; 40])?;
            Ok(1)
        }
        // Each guest runs on one worker; no other thread can enter its critical sections.
        "EnterCriticalSection" | "LeaveCriticalSection" | "DeleteCriticalSection" => Ok(0),
        "TlsAlloc" => {
            let slots = &mut cpu.get_data_mut().tls;
            if let Some((index, slot)) = slots.iter_mut().enumerate().find(|(_, s)| s.is_none()) {
                *slot = Some(0);
                return Ok(index as u64);
            }
            if slots.len() >= 64 {
                return Err("Windows TLS slots exhausted".into());
            }
            slots.push(Some(0));
            Ok(slots.len() as u64 - 1)
        }
        "TlsGetValue" => {
            let value = cpu
                .get_data()
                .tls
                .get(a as usize)
                .copied()
                .flatten()
                .ok_or("invalid TLS slot")?;
            cpu.get_data_mut().last_error = 0;
            Ok(value)
        }
        "TlsSetValue" => {
            let slot = cpu
                .get_data_mut()
                .tls
                .get_mut(a as usize)
                .ok_or("invalid TLS slot")?;
            if slot.is_none() {
                return Err("freed TLS slot".into());
            }
            *slot = Some(b);
            Ok(1)
        }
        "TlsFree" => {
            *cpu.get_data_mut()
                .tls
                .get_mut(a as usize)
                .ok_or("invalid TLS slot")? = None;
            Ok(1)
        }
        "IsDebuggerPresent" | "IsProcessorFeaturePresent" | "SetUnhandledExceptionFilter" => Ok(0),
        "GetStartupInfoW" => {
            cpu.mem_write(a, &[0; 104])?;
            cpu.mem_write(a, &104u32.to_le_bytes())?;
            Ok(0)
        }
        "GetModuleHandleW" | "LoadLibraryExW" => {
            let module = wide(cpu, a)?.to_ascii_lowercase();
            if module.is_empty() {
                Ok(0x7c800000)
            } else if ["kernel32.dll", "kernelbase.dll"].contains(&module.as_str()) {
                Ok(0x100)
            } else {
                cpu.get_data_mut().last_error = 126;
                Ok(0)
            }
        }
        "GetProcAddress" => {
            let symbol = ansi(cpu, b)?;
            if let Some((&ptr, _)) = cpu.get_data().imports.iter().find(|(_, n)| **n == symbol) {
                Ok(ptr)
            } else {
                cpu.get_data_mut().last_error = 127;
                Ok(0)
            }
        }
        "FreeLibrary" => Ok(1),
        "GetACP" | "GetOEMCP" => Ok(1252),
        "IsValidCodePage" => Ok(u64::from(a == 1252 || a == 65001)),
        "GetStdHandle" => Ok(u64::MAX),
        "GetModuleFileNameA" => {
            let path = b"C:\\Apple\\CoreADI.dll\0";
            if c < path.len() as u64 {
                return Err("module path buffer too small".into());
            }
            cpu.mem_write(b, path)?;
            Ok(path.len() as u64 - 1)
        }
        "GetCommandLineA" | "GetCommandLineW" | "GetEnvironmentStringsW" => Ok(SCRATCH + 0x80000),
        "FreeEnvironmentStringsW" => Ok(1),
        "GetCPInfo" => {
            cpu.mem_write(b, &[0; 20])?;
            cpu.mem_write(b, &1u32.to_le_bytes())?;
            cpu.mem_write(b + 4, b"?")?;
            Ok(1)
        }
        "MultiByteToWideChar" => {
            let d = d as u32 as u64;
            let bytes = if d as u32 == u32::MAX {
                let mut v = Vec::new();
                for offset in 0..4096 {
                    let byte = cpu.mem_read_as_vec(c + offset, 1)?[0];
                    v.push(byte);
                    if byte == 0 {
                        break;
                    }
                }
                if v.last() != Some(&0) {
                    return Err("unterminated Windows string".into());
                }
                v
            } else {
                if d > 4096 {
                    return Err("Windows conversion too large".into());
                }
                cpu.mem_read_as_vec(c, d as usize)?
            };
            let value = if a == 65001 {
                String::from_utf8(bytes).map_err(|_| "invalid UTF-8")?
            } else if a == 0 || a == 1252 {
                bytes.iter().map(|&b| cp1252(b)).collect()
            } else {
                return Err("unsupported codepage".into());
            };
            let output: Vec<u16> = value.encode_utf16().collect();
            let ptr = arg(cpu, 4)?;
            let capacity = arg(cpu, 5)? as u32 as u64;
            if capacity != 0 {
                if capacity < output.len() as u64 {
                    cpu.get_data_mut().last_error = 122;
                    return Ok(0);
                }
                let bytes: Vec<u8> = output.iter().flat_map(|c| c.to_le_bytes()).collect();
                cpu.mem_write(ptr, &bytes)?;
            }
            Ok(output.len() as u64)
        }
        "LCMapStringW" => {
            let count = d as u32 as u64;
            if count > 4096 || (b != 0x100 && b != 0x200) {
                return Err("unsupported locale mapping".into());
            }
            let bytes = cpu.mem_read_as_vec(c, count as usize * 2)?;
            let units: Vec<u16> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect();
            let input = String::from_utf16(&units).map_err(|_| "invalid UTF-16")?;
            let output: Vec<u16> = if b == 0x100 {
                input.to_lowercase()
            } else {
                input.to_uppercase()
            }
            .encode_utf16()
            .collect();
            let ptr = arg(cpu, 4)?;
            let capacity = arg(cpu, 5)? as u32 as u64;
            if capacity != 0 {
                if capacity < output.len() as u64 {
                    cpu.get_data_mut().last_error = 122;
                    return Ok(0);
                }
                let bytes: Vec<u8> = output.iter().flat_map(|c| c.to_le_bytes()).collect();
                cpu.mem_write(ptr, &bytes)?;
            }
            Ok(output.len() as u64)
        }
        "WideCharToMultiByte" => {
            let count = d as u32 as u64;
            let input = if count == u32::MAX as u64 {
                let mut s = wide(cpu, c)?;
                s.push('\0');
                s
            } else {
                if count > 4096 {
                    return Err("conversion too large".into());
                }
                let bytes = cpu.mem_read_as_vec(c, count as usize * 2)?;
                let units: Vec<u16> = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect();
                if b & 0x80 != 0 {
                    String::from_utf16(&units).map_err(|_| "invalid UTF-16")?
                } else {
                    String::from_utf16_lossy(&units)
                }
            };
            let mut substituted = false;
            let output: Vec<u8> = if a == 65001 {
                input.as_bytes().to_vec()
            } else if a == 0 || a == 1252 {
                input
                    .chars()
                    .map(|c| {
                        (0..=255u8).find(|&b| cp1252(b) == c).unwrap_or_else(|| {
                            substituted = true;
                            b'?'
                        })
                    })
                    .collect()
            } else {
                return Err("unsupported codepage".into());
            };
            let ptr = arg(cpu, 4)?;
            let capacity = arg(cpu, 5)? as u32 as u64;
            if capacity != 0 {
                if capacity < output.len() as u64 {
                    let end = if a == 65001 {
                        input.floor_char_boundary(capacity as usize)
                    } else {
                        capacity as usize
                    };
                    cpu.mem_write(ptr, &output[..end])?;
                    cpu.get_data_mut().last_error = 122;
                    return Ok(0);
                }
                cpu.mem_write(ptr, &output)?;
            }
            let used_default = arg(cpu, 7)?;
            if used_default != 0 {
                cpu.mem_write(used_default, &u32::from(substituted).to_le_bytes())?;
            }
            Ok(output.len() as u64)
        }
        "GetStringTypeW" => {
            if a != 1 || c > 4096 {
                return Err("unsupported character type request".into());
            }
            for i in 0..c {
                let raw = read_u16(cpu, b + i * 2)?;
                let ch = char::from_u32(raw as u32).ok_or("invalid codepoint")?;
                let flags: u16 = u16::from(ch.is_uppercase())
                    | (u16::from(ch.is_lowercase()) << 1)
                    | (u16::from(ch.is_ascii_digit()) << 2)
                    | (u16::from(ch.is_whitespace()) << 3)
                    | (u16::from(ch.is_ascii_punctuation()) << 4)
                    | (u16::from(ch.is_control()) << 5)
                    | (u16::from(ch == ' ' || ch == '\t') << 6)
                    | (u16::from(ch.is_ascii_hexdigit()) << 7)
                    | (u16::from(ch.is_alphabetic()) << 8)
                    | 0x200;
                cpu.mem_write(d + i * 2, &flags.to_le_bytes())?;
            }
            Ok(1)
        }
        _ => Err(Error::UnsupportedImport(name.into())),
    }
}

fn cp1252(byte: u8) -> char {
    const SPECIAL: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž',
        '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}',
        'ž', 'Ÿ',
    ];
    if (0x80..0xa0).contains(&byte) {
        SPECIAL[(byte - 0x80) as usize]
    } else {
        char::from(byte)
    }
}

fn write_wide(cpu: &mut Cpu<'_>, ptr: u64, value: &str) -> Result<(), Error> {
    let units: Vec<_> = value.encode_utf16().chain([0]).collect();
    if units.len() > 260 {
        return Err("Windows path exceeds MAX_PATH".into());
    }
    let bytes: Vec<_> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
    cpu.mem_write(ptr, &bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{BTreeMap, Binary, Cpu, REGS, Reg, Runtime, SCRATCH, STACK, STACK_SIZE, import};
    fn artificial() -> Runtime {
        let base = 0x7c800000;
        let mut memory = vec![0; 4096];
        memory[..6].copy_from_slice(&[0xb8, 1, 0, 0, 0, 0xc3]);
        memory[16..18].copy_from_slice(&[0xeb, 0xfe]);
        let binary = Binary {
            image: crate::pe::Image {
                memory,
                base,
                entry: base,
                imports: vec![],
                exports: BTreeMap::from([("vdfut768ig".into(), base)]),
            },
        };
        Runtime::new(&binary).unwrap()
    }
    fn arguments(cpu: &mut Cpu<'_>, args: &[u64]) {
        let stack = STACK + STACK_SIZE - 4096;
        cpu.reg_write(Reg::RSP, stack).unwrap();
        cpu.mem_write(stack, &[0; 256]).unwrap();
        for (i, reg) in REGS.iter().enumerate() {
            cpu.reg_write(*reg, args.get(i).copied().unwrap_or(0))
                .unwrap();
        }
        for (i, value) in args.iter().enumerate().skip(4) {
            cpu.mem_write(stack + 8 + i as u64 * 8, &value.to_le_bytes())
                .unwrap();
        }
    }
    #[test]
    fn bounded_execution_poisoning_and_unknown_imports() {
        let mut runtime = artificial();
        assert!(import(&mut runtime.cpu, "unsupported").is_err());
        assert!(runtime.invoke(0x7c800010, &[]).is_err());
        assert!(runtime.invoke(0x7c800000, &[]).is_err());
    }
    #[test]
    fn win64_dword_arguments_and_conversion_capacity() {
        let mut runtime = artificial();
        let cpu = &mut runtime.cpu;
        cpu.mem_write(SCRATCH, b"ABC").unwrap();
        arguments(cpu, &[1252, 0, SCRATCH, 3, 0, 0xdeadbeef00000000]);
        assert_eq!(import(cpu, "MultiByteToWideChar").unwrap(), 3);
        let source: Vec<_> = "AB€".encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
        cpu.mem_write(SCRATCH, &source).unwrap();
        cpu.mem_write(SCRATCH + 256, &[0x55; 8]).unwrap();
        arguments(cpu, &[65001, 0, SCRATCH, 3, SCRATCH + 256, 3, 0, 0]);
        assert_eq!(import(cpu, "WideCharToMultiByte").unwrap(), 0);
        assert_eq!(cpu.get_data().last_error, 122);
        assert_eq!(
            cpu.mem_read_as_vec(SCRATCH + 256, 4).unwrap(),
            [b'A', b'B', 0x55, 0x55]
        );
        let slot = import(cpu, "TlsAlloc").unwrap();
        arguments(cpu, &[slot]);
        assert_eq!(import(cpu, "TlsFree").unwrap(), 1);
        assert_eq!(import(cpu, "TlsAlloc").unwrap(), slot);
    }
}
