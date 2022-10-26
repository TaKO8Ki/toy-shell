use crate::builtins::{BuiltinCommandContext, BuiltinCommandError};
use crate::eval::evaluate_initializer;
use crate::fd_file::FdFile;
use crate::parser;
use crate::shell::Shell;
use crate::variable::Value;

use nix::sys::signal::{sigaction, SaFlags, SigAction, SigHandler, SigSet, Signal};
use nix::sys::termios::{tcgetattr, tcsetattr, SetArg::TCSADRAIN, Termios};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{execv, fork, getpid, setpgid, tcsetpgrp, ForkResult, Pid};
use std::cell::RefCell;
use std::ffi::CString;
use std::fmt;
use std::os::unix::io::RawFd;
use std::rc::Rc;
use tracing::debug;

/// The process execution environment.
#[derive(Debug, Copy, Clone)]
pub struct Context {
    pub stdin: RawFd,
    pub stdout: RawFd,
    pub stderr: RawFd,
    pub pgid: Option<Pid>,
    /// The process should be executed in background.
    pub background: bool,
    pub interactive: bool,
}

/// The exit status or reason why the command exited.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ExitStatus {
    ExitedWith(i32),
    Running(Pid),
    // TODO: support noexec
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct JobId(usize);

impl JobId {
    pub fn new(id: usize) -> JobId {
        JobId(id)
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// Represents a job.
/// See https://www.gnu.org/software/libc/manual/html_node/Implementing-a-Shell.html
pub struct Job {
    id: JobId,
    pub pgid: Pid,
    pub cmd: String,
    // TODO: Remove entries in shell.states on destruction.
    pub processes: Vec<Pid>,
    pub termios: RefCell<Option<Termios>>,
}

impl Job {
    pub fn new(id: JobId, pgid: Pid, cmd: String, processes: Vec<Pid>) -> Job {
        Job {
            id,
            pgid,
            cmd,
            processes,
            termios: RefCell::new(None),
        }
    }

    pub fn completed(&self, shell: &Shell) -> bool {
        self.processes.iter().all(|pid| {
            let state = shell.get_process_state(*pid).unwrap();
            matches!(state, ProcessState::Completed(_))
        })
    }

    pub fn stopped(&self, shell: &Shell) -> bool {
        self.processes.iter().all(|pid| {
            let state = shell.get_process_state(*pid).unwrap();
            matches!(state, ProcessState::Stopped(_))
        })
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ProcessState {
    Running,
    /// Contains the exit status.
    Completed(i32),
    /// Suspended (Ctrl-Z).
    Stopped(Pid),
}

pub fn run_in_foreground(shell: &mut Shell, job: &Rc<Job>) -> ProcessState {
    debug!("run_in_foreground");
    shell.last_fore_job = Some(job.clone());
    set_terminal_process_group(job.pgid);

    let status = wait_for_job(shell, job);

    // Save the current terminal status.
    job.termios
        .replace(Some(tcgetattr(0).expect("failed to tcgetattr")));

    // Go back into the shell.
    set_terminal_process_group(shell.shell_pgid);
    restore_terminal_attrs(shell.shell_termios.as_ref().unwrap());

    status
}

pub fn set_terminal_process_group(pgid: Pid) {
    tcsetpgrp(0, pgid).expect("failed to tcsetpgrp");
}

pub fn restore_terminal_attrs(termios: &Termios) {
    tcsetattr(0, TCSADRAIN, termios).expect("failed to tcsetattr");
}

pub fn wait_for_job(shell: &mut Shell, job: &Rc<Job>) -> ProcessState {
    loop {
        if job.completed(shell) || job.stopped(shell) {
            break;
        }

        wait_for_any_process(shell, false);
    }

    // Get the exit status of the last process.
    let state = shell
        .get_process_state(*job.processes.iter().last().unwrap())
        .cloned();

    match state {
        Some(ProcessState::Completed(_)) => {
            // Remove the job and processes from the list.
            destroy_job(shell, job);
            state.unwrap()
        }
        Some(ProcessState::Stopped(_)) => {
            smash_err!("[{}] Stopped: {}", job.id, job.cmd);
            state.unwrap()
        }
        _ => unreachable!(),
    }
}

pub fn wait_for_any_process(shell: &mut Shell, no_block: bool) -> Option<Pid> {
    let options = if no_block {
        WaitPidFlag::WUNTRACED | WaitPidFlag::WNOHANG
    } else {
        WaitPidFlag::WUNTRACED
    };

    let result = waitpid(None, Some(options));
    let (pid, state) = match result {
        Ok(WaitStatus::Exited(pid, status)) => {
            debug!("exited: pid={} status={}", pid, status);
            (pid, ProcessState::Completed(status))
        }
        Ok(WaitStatus::Signaled(pid, _signal, _)) => {
            // The `pid` process has been killed by `_signal`.
            (pid, ProcessState::Completed(-1))
        }
        Ok(WaitStatus::Stopped(pid, _signal)) => (pid, ProcessState::Stopped(pid)),
        Err(nix::errno::Errno::ECHILD) | Ok(WaitStatus::StillAlive) => {
            // No childs to be reported.
            return None;
        }
        status => {
            panic!("unexpected waitpid event: {:?}", status);
        }
    };

    shell.set_process_state(pid, state);
    Some(pid)
}

pub fn destroy_job(shell: &mut Shell, job: &Rc<Job>) {
    // TODO: support background jobs

    shell.jobs_mut().remove(&job.id).unwrap();

    if let Some(ref last_job) = shell.last_fore_job {
        if job.id == last_job.id {
            shell.last_fore_job = None;
        }
    }
}

pub fn wait_child(pid: Pid) -> anyhow::Result<i32> {
    let wait_status = waitpid(pid, None)?;
    match wait_status {
        WaitStatus::Exited(_, status) => Ok(status),
        // TODO: Handle errors.
        _ => {
            let err = anyhow::anyhow!("waitpid returned an unexpected value: {:?}", wait_status);

            debug!("waitpid: {}", err);
            Err(err)
        }
    }
}

pub fn run_internal_command(
    shell: &mut Shell,
    argv: &[String],
    stdin: RawFd,
    stdout: RawFd,
    stderr: RawFd,
    _redirects: &[parser::Redirection],
) -> anyhow::Result<ExitStatus> {
    let command = match crate::builtins::builtin_command(argv[0].as_str()) {
        Some(func) => func,
        _ => return Err(BuiltinCommandError::NotFound.into()),
    };

    // TODO: support redirections

    let result = command.run(&mut BuiltinCommandContext {
        argv,
        shell,
        stdin: FdFile::new(stdin),
        stdout: FdFile::new(stdout),
        stderr: FdFile::new(stderr),
    });

    Ok(result)
}

pub fn run_external_command(
    shell: &mut Shell,
    ctx: &Context,
    argv: Vec<String>,
    _redirects: &[parser::Redirection],
    assignments: &[parser::Assignment],
) -> anyhow::Result<ExitStatus> {
    // TODO: support redirections

    let argv0 = if argv[0].starts_with('/') || argv[0].starts_with("./") {
        CString::new(argv[0].as_str())?
    } else {
        match shell.path_table().lookup(&argv[0]) {
            Some(path) => CString::new(path)?,
            None => {
                smash_err!("command not found `{}`", argv[0]);
                return Ok(ExitStatus::ExitedWith(1));
            }
        }
    };

    let mut args = Vec::new();
    for arg in argv {
        args.push(CString::new(arg)?);
    }

    // Spawn a child.
    match unsafe { fork() }.expect("failed to fork") {
        ForkResult::Parent { child } => Ok(ExitStatus::Running(child)),
        ForkResult::Child => {
            // Create or join a process group.
            if ctx.interactive {
                let pid = getpid();
                let pgid = match ctx.pgid {
                    Some(pgid) => {
                        setpgid(pid, pgid).expect("failed to setpgid");
                        pgid
                    }
                    None => {
                        setpgid(pid, pid).expect("failed to setpgid");
                        pid
                    }
                };

                if !ctx.background {
                    set_terminal_process_group(pgid);
                    restore_terminal_attrs(shell.shell_termios.as_ref().unwrap());
                }

                // Accept job-control-related signals (refer https://www.gnu.org/software/libc/manual/html_node/Launching-Jobs.html)
                let action = SigAction::new(SigHandler::SigDfl, SaFlags::empty(), SigSet::empty());
                unsafe {
                    sigaction(Signal::SIGINT, &action).expect("failed to sigaction");
                    sigaction(Signal::SIGQUIT, &action).expect("failed to sigaction");
                    sigaction(Signal::SIGTSTP, &action).expect("failed to sigaction");
                    sigaction(Signal::SIGTTIN, &action).expect("failed to sigaction");
                    sigaction(Signal::SIGTTOU, &action).expect("failed to sigaction");
                    sigaction(Signal::SIGCHLD, &action).expect("failed to sigaction");
                }
            }

            for name in shell.exported_names() {
                if let Some(var) = shell.get(name) {
                    std::env::set_var(name, var.as_str());
                }
            }

            for assignment in assignments {
                let value = evaluate_initializer(shell, &assignment.initializer)
                    .expect("failed to evaluate the initializer");
                match value {
                    Value::String(s) => std::env::set_var(&assignment.name, s),
                    Value::Array(_) => {
                        eprintln!("Array assignments in a command is not supported.");
                        std::process::exit(1);
                    }
                }
            }

            let args: Vec<&std::ffi::CStr> = args.iter().map(|s| s.as_c_str()).collect();
            match execv(&argv0, &args) {
                Ok(_) => {
                    unreachable!();
                }
                Err(nix::errno::Errno::EACCES) => {
                    eprintln!("Failed to exec {:?} (EACCESS). chmod(1) may help.", argv0);
                    std::process::exit(1);
                }
                Err(err) => {
                    eprintln!("Failed to exec {:?} ({})", argv0, err);
                    std::process::exit(1);
                }
            }
        }
    }
}
