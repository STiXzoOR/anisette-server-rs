// SPDX-License-Identifier: GPL-2.0-only
// Memory layout and import model adapted from os-findmy-e macOS-ADI-emulator/harness.py.
use crate::{Binary, Error};
use anisette::memory::Heap;
use std::{collections::BTreeMap, fmt};
use unicorn_engine::{
    RegisterX86, Unicorn,
    unicorn_const::{Arch, Mode, Prot},
};

pub(crate) const RETURN: u64 = 0x40000000;
pub(crate) const HEAP: u64 = 0x40010000;
const HEAP_SIZE: u64 = 0x01000000;
const STACK: u64 = 0x50000000;
const STACK_SIZE: u64 = 0x00200000;
const STUB: u64 = 0x60000000;
const STUB_SIZE: u64 = 0x20000;
const DATA: u64 = 0x60020000;
const SCRATCH: u64 = 0x60040000;
const OBJECT: u64 = 0x60080000;
const ARENA_SIZE: u64 = 0x40000;
const ARGUMENTS: [RegisterX86; 6] = [
    RegisterX86::RDI,
    RegisterX86::RSI,
    RegisterX86::RDX,
    RegisterX86::RCX,
    RegisterX86::R8,
    RegisterX86::R9,
];
const DATA_SYMBOLS: &[&str] = &[
    "stack_chk_guard",
    "kCFAllocatorDefault",
    "kCFBooleanFalse",
    "kCFBooleanTrue",
    "kCFErrorDomainOSStatus",
    "kCFURLFileSecurityKey",
    "kCFURLIsDirectoryKey",
    "kCFURLIsHiddenKey",
    "kCFURLTypeIdentifierKey",
    "kUTTypePlainText",
    "kCFAllocatorNull",
    "kCFRunLoopCommonModes",
    "NSConcreteGlobalBlock",
    "NSConcreteStackBlock",
    "CFConstantStringClassReference",
    "dispatch_main_q",
    "os_log_default",
    "mach_task_self_",
    "xpc_type_dictionary",
    "xpc_type_error",
    "xpc_error_connection_invalid",
];

pub(crate) struct Env {
    pub symbols: BTreeMap<u64, String>,
    pub cells: BTreeMap<String, u64>,
    pub heap: Heap,
    pub scratch: u64,
    pub object: u64,
    pub fault: Option<Error>,
    pub runloop: bool,
    pub listener_block: Option<u64>,
    pub peer_block: Option<u64>,
    pub objects: BTreeMap<u64, crate::xpc::Object>,
    pub reply: Option<BTreeMap<String, Vec<u8>>>,
    pub fs: crate::filesystem::FileSystem,
    pub errno: u64,
    pub host_uuid: [u8; 16],
}
impl Env {
    fn new() -> Result<Self, Error> {
        Ok(Self {
            symbols: BTreeMap::new(),
            cells: BTreeMap::new(),
            heap: Heap::new(HEAP, HEAP_SIZE)?,
            scratch: SCRATCH,
            object: OBJECT,
            fault: None,
            runloop: false,
            listener_block: None,
            peer_block: None,
            objects: BTreeMap::new(),
            reply: None,
            fs: Default::default(),
            errno: 0,
            host_uuid: [0; 16],
        })
    }
    pub fn scratch(&mut self, size: u64) -> Result<u64, Error> {
        bump(&mut self.scratch, SCRATCH + ARENA_SIZE, size)
    }
    pub fn object(&mut self, size: u64) -> Result<u64, Error> {
        bump(&mut self.object, OBJECT + ARENA_SIZE, size)
    }
}
fn bump(cursor: &mut u64, end: u64, size: u64) -> Result<u64, Error> {
    let size = size
        .checked_add(15)
        .ok_or(Error::Execution("arena allocation overflow"))?
        & !15;
    let next = cursor
        .checked_add(size)
        .filter(|n| *n <= end)
        .ok_or(Error::Execution("emulated arena exhausted"))?;
    let address = *cursor;
    *cursor = next;
    Ok(address)
}
pub(crate) type Cpu<'a> = Unicorn<'a, Env>;
pub struct Runtime {
    pub(crate) cpu: Cpu<'static>,
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
    pub(crate) fn ensure_usable(&self) -> Result<(), Error> {
        if self.poisoned {
            return Err(Error::Execution("runtime unusable after failed execution"));
        }
        Ok(())
    }

    /// Creates a macOS emulator from a verified binary without booting the daemon.
    ///
    /// # Errors
    /// Returns an error if randomness, guest memory setup, import binding, or emulator initialization fails.
    pub fn new(binary: &Binary) -> Result<Self, Error> {
        let mut cpu = Unicorn::new_with_data(Arch::X86, Mode::MODE_64, Env::new()?)?;
        for (address, size, prot) in [
            (RETURN, 4096, Prot::READ | Prot::EXEC),
            (HEAP, HEAP_SIZE, Prot::READ | Prot::WRITE),
            (STACK, STACK_SIZE, Prot::READ | Prot::WRITE),
            (STUB, STUB_SIZE, Prot::READ | Prot::WRITE),
            (DATA, STUB_SIZE, Prot::READ | Prot::WRITE),
            (SCRATCH, ARENA_SIZE, Prot::READ | Prot::WRITE),
            (OBJECT, ARENA_SIZE, Prot::READ | Prot::WRITE),
        ] {
            cpu.mem_map(address, size, prot)?;
        }
        for segment in &binary.image().segments {
            if segment.name == "__PAGEZERO" || segment.size == 0 {
                continue;
            }
            cpu.mem_map(
                segment.address,
                segment.size.div_ceil(4096) * 4096,
                Prot::ALL,
            )?;
            cpu.mem_write(
                segment.address,
                &binary.bytes()[segment.file_offset..segment.file_offset + segment.file_size],
            )?;
        }
        let errno = cpu.get_data_mut().scratch(8)?;
        cpu.get_data_mut().errno = errno;
        getrandom::fill(&mut cpu.get_data_mut().host_uuid)
            .map_err(|_| Error::Execution("host random source failed"))?;
        cpu.mem_write(STUB, &vec![0xc3; STUB_SIZE as usize])?;
        cpu.mem_protect(STUB, STUB_SIZE, Prot::READ | Prot::EXEC)?;
        let mut addresses = BTreeMap::new();
        for binding in &binary.image().bindings {
            let name = binding.symbol.trim_start_matches('_');
            if !addresses.contains_key(name) {
                let index = addresses.len() as u64;
                if index * 16 >= STUB_SIZE {
                    return Err(Error::Execution("too many imports"));
                }
                let address = if DATA_SYMBOLS.contains(&name) {
                    let cell = DATA + index * 16;
                    cpu.mem_write(cell, &(OBJECT + index * 16).to_le_bytes())?;
                    cpu.get_data_mut().cells.insert(name.to_owned(), cell);
                    cell
                } else {
                    let address = STUB + index * 16;
                    cpu.get_data_mut().symbols.insert(address, name.to_owned());
                    address
                };
                addresses.insert(name.to_owned(), address);
            }
            cpu.mem_write(
                binding.address,
                &addresses[name]
                    .wrapping_add_signed(binding.addend)
                    .to_le_bytes(),
            )?;
        }
        cpu.get_data_mut().object = OBJECT + (addresses.len() as u64) * 16;
        cpu.add_code_hook(STUB, STUB + STUB_SIZE - 1, |cpu, address, _| {
            let Some(name) = cpu.get_data().symbols.get(&address).cloned() else {
                cpu.get_data_mut().fault = Some(Error::Execution("invalid import stub"));
                let _ = cpu.emu_stop();
                return;
            };
            match import(cpu, &name) {
                Ok(Some(value)) => {
                    if let Err(error) = cpu.reg_write(RegisterX86::RAX, value) {
                        cpu.get_data_mut().fault = Some(error.into());
                        let _ = cpu.emu_stop();
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    cpu.get_data_mut().fault = Some(error);
                    let _ = cpu.emu_stop();
                }
            }
        })?;
        Ok(Self {
            cpu,
            poisoned: false,
        })
    }
    pub(crate) fn invoke(&mut self, address: u64, args: &[u64], boot: bool) -> Result<u64, Error> {
        self.invoke_with_budget(address, args, boot, 5_000_000, 50_000_000)
    }
    fn invoke_with_budget(
        &mut self,
        address: u64,
        args: &[u64],
        boot: bool,
        micros: u64,
        instructions: usize,
    ) -> Result<u64, Error> {
        self.ensure_usable()?;
        if args.len() > 6 || micros == 0 || instructions == 0 {
            return Err(Error::Execution("invalid invocation budget or arguments"));
        }
        self.poisoned = true;
        for (i, reg) in ARGUMENTS.iter().enumerate() {
            self.cpu
                .reg_write(*reg, args.get(i).copied().unwrap_or(0))?;
        }
        let rsp = STACK + STACK_SIZE - 4096;
        self.cpu.mem_write(rsp, &RETURN.to_le_bytes())?;
        self.cpu.reg_write(RegisterX86::RSP, rsp)?;
        self.cpu.reg_write(RegisterX86::RBP, 0)?;
        self.cpu.get_data_mut().fault = None;
        self.cpu.get_data_mut().runloop = false;
        self.cpu.emu_start(address, RETURN, micros, instructions)?;
        if let Some(error) = self.cpu.get_data_mut().fault.take() {
            return Err(error);
        }
        if !(self.cpu.reg_read(RegisterX86::RIP)? == RETURN || boot && self.cpu.get_data().runloop)
        {
            return Err(Error::Execution("execution budget exhausted"));
        }
        self.poisoned = false;
        Ok(self.cpu.reg_read(RegisterX86::RAX)?)
    }
    /// Starts the pinned daemon, reaches its run loop, then installs a synthetic XPC peer.
    ///
    /// # Errors
    /// Returns an error if execution fails or the daemon does not establish its run loop and XPC handlers.
    pub fn boot(&mut self) -> Result<(), Error> {
        let path = self.cpu.get_data_mut().scratch(32)?;
        self.cpu.mem_write(path, b"/usr/libexec/adid\0")?;
        let argv = self.cpu.get_data_mut().scratch(16)?;
        self.cpu.mem_write(argv, &path.to_le_bytes())?;
        self.cpu.mem_write(argv + 8, &0u64.to_le_bytes())?;
        self.invoke(0x100030eb0, &[1, argv], true)?;
        if !self.cpu.get_data().runloop {
            return Err(Error::Execution("daemon did not reach run loop"));
        }
        let peer = crate::xpc::object(&mut self.cpu, crate::xpc::Object::Peer)?;
        let block = self
            .cpu
            .get_data()
            .listener_block
            .ok_or(Error::Execution("missing listener block"))?;
        self.invoke_block(block, peer)?;
        if self.cpu.get_data().peer_block.is_none() {
            return Err(Error::Execution("missing peer block"));
        }
        Ok(())
    }
    fn invoke_block(&mut self, block: u64, argument: u64) -> Result<(), Error> {
        let function = read_u64(&self.cpu, block + 16)?;
        self.invoke(function, &[block, argument], false)?;
        Ok(())
    }
    pub(crate) fn call(&mut self, code: u32, body: &[u8]) -> Result<(i32, Vec<u8>), Error> {
        if body.len() > 128 * 1024 {
            return Err(Error::Execution("ADI request too large"));
        }
        let block = self
            .cpu
            .get_data()
            .peer_block
            .ok_or(Error::Execution("daemon not booted"))?;
        let scratch = self.cpu.get_data().scratch;
        let cursor = self.cpu.get_data().object;
        let saved = self.cpu.get_data().objects.clone();
        let result = (|| {
            let mut request = Vec::new();
            for word in [1u32, code, 1] {
                request.extend(word.to_be_bytes());
            }
            request.extend(body);
            self.cpu.get_data_mut().reply = None;
            let message = crate::xpc::object(
                &mut self.cpu,
                crate::xpc::Object::Dictionary(BTreeMap::from([("data".into(), request)])),
            )?;
            self.invoke_block(block, message)?;
            let reply = self
                .cpu
                .get_data_mut()
                .reply
                .take()
                .ok_or(Error::Execution("missing XPC reply"))?;
            let data = reply
                .get("data")
                .ok_or(Error::Execution("missing ADI reply data"))?;
            let header = data.get(..4).ok_or(Error::Execution("short ADI reply"))?;
            let status = i32::from_be_bytes([header[0], header[1], header[2], header[3]]);
            Ok((status, data[4..].to_vec()))
        })();
        let env = self.cpu.get_data_mut();
        env.scratch = scratch;
        env.object = cursor;
        env.objects = saved;
        env.reply = None;
        result
    }
}
pub(crate) fn read_u64(cpu: &Cpu<'_>, address: u64) -> Result<u64, Error> {
    let mut bytes = [0u8; 8];
    cpu.mem_read(address, &mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}
pub(crate) fn argument(cpu: &Cpu<'_>, index: usize) -> Result<u64, Error> {
    Ok(cpu.reg_read(ARGUMENTS[index])?)
}
fn import(cpu: &mut Cpu<'_>, name: &str) -> Result<Option<u64>, Error> {
    if let Some(value) = crate::xpc::call(cpu, name)? {
        return Ok(Some(value));
    }
    if let Some(value) = crate::stubs::call(cpu, name)? {
        return Ok(Some(value));
    }
    let value = match name {
        // The guest stack is fully mapped; this probe must preserve RAX (allocation size).
        "chkstk_darwin" => return Ok(None),
        "CFRunLoopRun" => {
            cpu.get_data_mut().runloop = true;
            cpu.emu_stop()?;
            return Ok(None);
        }
        "os_transaction_create"
        | "CFFileDescriptorCreate"
        | "CFFileDescriptorCreateRunLoopSource" => cpu.get_data_mut().object(64)?,
        "os_release"
        | "pthread_set_qos_class_self_np"
        | "ptrace"
        | "sched_yield"
        | "os_log_type_enabled"
        | "os_log_impl"
        | "xpc_connection_set_target_queue"
        | "xpc_connection_activate"
        | "xpc_connection_resume"
        | "xpc_connection_cancel"
        | "xpc_transaction_begin"
        | "xpc_transaction_end"
        | "xpc_release"
        | "kevent"
        | "signal"
        | "CFRunLoopAddSource"
        | "CFFileDescriptorEnableCallBacks"
        | "CFRelease" => 0,
        "arc4random" => {
            let mut bytes = [0u8; 4];
            getrandom::fill(&mut bytes)
                .map_err(|_| Error::Execution("host random source failed"))?;
            u64::from(u32::from_le_bytes(bytes))
        }
        "arc4random_buf" => {
            let p = argument(cpu, 0)?;
            let n = argument(cpu, 1)?;
            if n > HEAP_SIZE {
                return Err(Error::Execution("random buffer too large"));
            }
            let mut bytes = vec![0; n as usize];
            getrandom::fill(&mut bytes)
                .map_err(|_| Error::Execution("host random source failed"))?;
            cpu.mem_write(p, &bytes)?;
            0
        }
        "getpid" => 4242,
        "getuid" => 501,
        "umask" => 0o022,
        "kqueue" => 0x34,
        "sysconf" => 4096,
        "sysctlbyname" => {
            let p = argument(cpu, 1)?;
            let lenp = argument(cpu, 2)?;
            if p != 0 && lenp != 0 {
                let n = read_u64(cpu, lenp)?.min(8);
                cpu.mem_write(p, &vec![0; n as usize])?;
            }
            0
        }
        "mmap" => {
            let n = argument(cpu, 1)?;
            let rounded = n
                .checked_add(4095)
                .ok_or(Error::Execution("mmap overflow"))?
                & !4095;
            let p = cpu.get_data_mut().heap.allocate(rounded, 4096)?;
            cpu.mem_write(p, &vec![0; rounded as usize])?;
            p
        }
        "munmap" => {
            let p = argument(cpu, 0)?;
            cpu.get_data_mut().heap.release(p)?;
            0
        }
        "gettimeofday" => {
            let p = argument(cpu, 0)?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| Error::Execution("host clock before epoch"))?;
            if p != 0 {
                cpu.mem_write(p, &now.as_secs().to_le_bytes())?;
                cpu.mem_write(p + 8, &u64::from(now.subsec_micros()).to_le_bytes())?;
            }
            0
        }
        "malloc" => {
            let size = argument(cpu, 0)?;
            cpu.get_data_mut().heap.allocate(size, 16)?
        }
        "free" => {
            let address = argument(cpu, 0)?;
            cpu.get_data_mut().heap.release(address)?;
            0
        }
        "pthread_rwlock_init"
        | "pthread_rwlock_rdlock"
        | "pthread_rwlock_wrlock"
        | "pthread_rwlock_unlock"
        | "pthread_rwlock_destroy" => 0,
        "CFRunLoopGetMain" | "CFRunLoopGetCurrent" => cpu.get_data_mut().object(32)?,
        _ => return Err(Error::UnsupportedImport(name.to_owned())),
    };
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::{Arch, Env, Error, Mode, Prot, Runtime, STACK, STACK_SIZE, Unicorn, bump};
    #[test]
    fn bounded_arenas_reject_overflow() {
        let mut cursor = 16;
        assert!(bump(&mut cursor, 32, 17).is_err());
        assert_eq!(cursor, 16);
        assert!(bump(&mut cursor, 32, u64::MAX).is_err());
        assert_eq!(bump(&mut cursor, 32, 16).unwrap(), 16);
    }
    #[test]
    fn execution_budget_stops_loop_and_poisoned_runtime_cannot_resume() {
        let mut cpu =
            Unicorn::new_with_data(Arch::X86, Mode::MODE_64, Env::new().unwrap()).unwrap();
        cpu.mem_map(STACK, STACK_SIZE, Prot::ALL).unwrap();
        cpu.mem_map(0x1000, 4096, Prot::ALL).unwrap();
        cpu.mem_write(0x1000, &[0xeb, 0xfe]).unwrap();
        let mut runtime = Runtime {
            cpu,
            poisoned: false,
        };
        assert!(matches!(
            runtime.invoke_with_budget(0x1000, &[], false, 100_000, 100),
            Err(Error::Execution("execution budget exhausted"))
        ));
        assert!(matches!(
            runtime.invoke(0x1000, &[], false),
            Err(Error::Execution("runtime unusable after failed execution"))
        ));
        assert!(
            runtime.export_state([0; 16]).is_err(),
            "poisoned runtime must not export state"
        );
    }
}
