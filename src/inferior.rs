use nix::sys::ptrace;
use nix::sys::signal;
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use std::collections::HashMap;
use std::mem::size_of;
use nix::unistd::Pid;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command};
use crate::dwarf_data::DwarfData;

pub enum Status {
    /// Indicates inferior stopped. Contains the signal that stopped the process, as well as the
    /// current instruction pointer that it is stopped at.
    Stopped(signal::Signal, usize),

    /// Indicates inferior exited normally. Contains the exit status code.
    Exited(i32),

    /// Indicates the inferior exited due to a signal. Contains the signal that killed the
    /// process.
    Signaled(signal::Signal),
}

/// This function calls ptrace with PTRACE_TRACEME to enable debugging on a process. You should use
/// pre_exec with Command to call this in the child process.
fn child_traceme() -> Result<(), std::io::Error> {
    ptrace::traceme().or(Err(std::io::Error::new(
        std::io::ErrorKind::Other,
        "ptrace TRACEME failed",
    )))
}

pub struct Inferior {
    child: Child,
    bp_to_original_byte: HashMap<usize, u8>
}

impl Inferior {
    /// Attempts to start a new inferior process. Returns Some(Inferior) if successful, or None if
    /// an error is encountered.
    pub fn new(target: &str, args: &Vec<String>, break_points: &mut Vec<usize>) -> Option<Inferior> {
        // TODO: implement me!
        let mut command = Command::new(target);
        command.args(args);
        unsafe {
            command.pre_exec(|| {
                child_traceme()
            });
        }
        let child = command.spawn().expect("spawn shit!");
        let child_pid = Pid::from_raw(child.id() as i32);
        let status = waitpid(Some(child_pid), None).expect("wait shit!");
        match status {
            WaitStatus::Exited(_, _) => {
                None
            },
            WaitStatus::Signaled(_, _, _) => {
                None
            },
            WaitStatus::Stopped(_, signal) => {
                match signal {
                    signal::SIGTRAP => {
                        let mut ret_inf = Inferior{child, bp_to_original_byte: HashMap::new()};
                        ret_inf.install_break_points(break_points).ok()?;
                        Some(ret_inf)
                    },
                    _ => None
                }
            },
            WaitStatus::PtraceEvent(_, _, _) => {
                None
            },
            WaitStatus::PtraceSyscall(_) => {
                None
            },
            WaitStatus::Continued(_) => {
                None
            },
            WaitStatus::StillAlive => {
                None
            }
        }
    }

    /// Returns the pid of this inferior.
    pub fn pid(&self) -> Pid {
        nix::unistd::Pid::from_raw(self.child.id() as i32)
    }

    /// Calls waitpid on this inferior and returns a Status to indicate the state of the process
    /// after the waitpid call.
    pub fn wait(&self, options: Option<WaitPidFlag>) -> Result<Status, nix::Error> {
        Ok(match waitpid(self.pid(), options)? {
            WaitStatus::Exited(_pid, exit_code) => Status::Exited(exit_code),
            WaitStatus::Signaled(_pid, signal, _core_dumped) => Status::Signaled(signal),
            WaitStatus::Stopped(_pid, signal) => {
                let regs = ptrace::getregs(self.pid())?;
                Status::Stopped(signal, regs.rip as usize)
            }
            other => panic!("waitpid returned unexpected status: {:?}", other),
        })
    }

    pub fn continue_running(&mut self, break_points: &mut Vec<usize>) -> Result<Status, nix::Error> {
        self.install_break_points(break_points)?;
        let inf_pid = self.pid();
        let mut regs = ptrace::getregs(inf_pid)?;
        let possible_bp_addr = (regs.rip - 1) as usize;
        if let Some(origin_byte) = self.bp_to_original_byte.get(&possible_bp_addr) {
            self.write_byte(possible_bp_addr, *origin_byte)?;
            regs.rip -= 1;
            ptrace::setregs(inf_pid, regs)?;
            ptrace::step(inf_pid, None)?;
            // println!("after step");
            self.write_byte(possible_bp_addr, 0xcc)?;
            // println!("after write byte");
        }
        ptrace::cont(inf_pid, None)?;
        self.wait(None)
    }

    pub fn kill(&mut self) -> Vec<usize> {
        println!("process {} being killed", self.child.id());
        self.child.kill().expect("failed to kill process");
        waitpid(self.pid(), None).expect("failed to reaping killed process");
        self.bp_to_original_byte.keys().map(|k| *k).collect()
    }
    pub fn print_backtrace(&self, debug_data: 
        &DwarfData) -> Result<(), nix::Error> {
        let regs = ptrace::getregs(self.pid())?;
        let mut rip = regs.rip;
        let mut rbp = regs.rbp;
        loop {
            let function_name = print_function_line(rip as usize, debug_data)?;
            if function_name == "main" {
                break;
            }
            rip = ptrace::read(self.pid(), (rbp + 8) as ptrace::AddressType)? as u64;
            rbp = ptrace::read(self.pid(), rbp as ptrace::AddressType)? as u64;
        }
        Ok(())
    }
    fn write_byte(&mut self, addr: usize, val: u8) -> Result<u8, nix::Error> {
        let aligned_addr = align_addr_to_word(addr);
        let byte_offset = addr - aligned_addr;
        let word = ptrace::read(self.pid(), aligned_addr as ptrace::AddressType)? as u64;
        let orig_byte = (word >> 8 * byte_offset) & 0xff;
        let masked_word = word & !(0xff << 8 * byte_offset);
        let updated_word = masked_word | ((val as u64) << 8 * byte_offset);
        ptrace::write(
            self.pid(),
            aligned_addr as ptrace::AddressType,
            updated_word as *mut std::ffi::c_void,
        )?;
        Ok(orig_byte as u8)
    }
    fn install_break_points(&mut self, break_points: &mut Vec<usize>) -> Result<(), nix::Error> {
        while let Some(addr) = break_points.pop() {
            let origin_byte = self.write_byte(addr, 0xcc)?;
            self.bp_to_original_byte.insert(addr, origin_byte);
        }
        Ok(())
    }
}

fn print_function_line(rip: usize, debug_data: &DwarfData) -> Result<String, nix::Error> {
    if let Some(line) = debug_data.get_line_from_addr(rip as usize) {
        if let Some(function_name) = debug_data.get_function_from_addr(rip as usize) {
            println!("{} ({})", function_name, line);
            Ok(function_name)
        } else {
            println!("no function name");
            Err(nix::Error::Sys(nix::errno::Errno::UnknownErrno))
        }
    } else {
        println!("no line number");
        Err(nix::Error::Sys(nix::errno::Errno::UnknownErrno))
    }
}

fn align_addr_to_word(addr: usize) -> usize {
    addr & (-(size_of::<usize>() as isize) as usize)
}