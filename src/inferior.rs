use nix::sys::ptrace;
use nix::sys::signal;
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command};

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
}

impl Inferior {
    /// Attempts to start a new inferior process. Returns Some(Inferior) if successful, or None if
    /// an error is encountered.
    pub fn new(target: &str, args: &Vec<String>) -> Option<Inferior> {
        // TODO: implement me!
        // println!(
        //     "Inferior::new not implemented! target={}, args={:?}",
        //     target, args
        // );
        let mut command = Command::new(target);
        command.args(args);
        unsafe {
            command.pre_exec(|| {
                ptrace::traceme().expect("traceme shit!");
                Ok(())
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
                    signal::SIGTRAP => Some(Inferior{child}),
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

    pub fn continue_running(&mut self) -> Result<Status, nix::Error> {
        ptrace::cont(self.pid(), None)?;
        self.wait(None)
    }

    pub fn kill(&mut self) {
        println!("process {} being killed", self.child.id());
        self.child.kill().expect("failed to kill process");
        waitpid(self.pid(), None).expect("failed to reaping killed process");
    }
}
