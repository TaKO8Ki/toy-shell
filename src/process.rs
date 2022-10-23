use crate::shell::Shell;

use nix::sys::signal::{kill, sigaction, SaFlags, SigAction, SigHandler, SigSet, Signal};
use nix::sys::termios::{tcgetattr, tcsetattr, SetArg::TCSADRAIN, Termios};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{close, dup2, execv, fork, getpid, setpgid, tcsetpgrp, ForkResult, Pid};
use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt;
use std::rc::Rc;
use tracing::debug;

/// The exit status or reason why the command exited.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ExitStatus {
    ExitedWith(i32),
    Running(Pid /* pgid */),
    Break,
    Continue,
    Return,
    // The command is not executed because of `noexec`.
    NoExec,
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

    #[inline]
    pub fn id(&self) -> JobId {
        self.id
    }

    #[inline]
    pub fn cmd(&self) -> &str {
        self.cmd.as_str()
    }

    pub fn state(&self, shell: &Shell) -> &'static str {
        if self.completed(shell) {
            "done"
        } else if self.stopped(shell) {
            "stopped"
        } else {
            "running"
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

pub fn run_in_foreground(shell: &mut Shell, job: &Rc<Job>, sigcont: bool) -> ProcessState {
    shell.last_fore_job = Some(job.clone());
    // shell.background_jobs_mut().remove(job);
    set_terminal_process_group(job.pgid);

    // if sigcont {
    //     if let Some(ref termios) = *job.termios.borrow() {
    //         restore_terminal_attrs(termios);
    //     }
    //     kill_process_group(job.pgid, Signal::SIGCONT).expect("failed to kill(SIGCONT)");
    //     debug!("sent sigcont");
    // }

    // Wait for the job to exit or stop.
    let status = wait_for_job(shell, job);

    // Save the current terminal status.
    job.termios
        .replace(Some(tcgetattr(0).expect("failed to tcgetattr")));

    // Go back into the shell.
    set_terminal_process_group(shell.shell_pgid);
    restore_terminal_attrs(shell.shell_termios.as_ref().unwrap());

    status
}

fn set_terminal_process_group(pgid: Pid) {
    tcsetpgrp(0, pgid).expect("failed to tcsetpgrp");
}

fn restore_terminal_attrs(termios: &Termios) {
    tcsetattr(0, TCSADRAIN, termios).expect("failed to tcsetattr");
}

// fn kill_process_group(pgid: Pid, signal: Signal) -> anyhow::Result<()> {
//     kill(Pid::from_raw(-pgid.as_raw()), signal).map(Error::into)
// }

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
    // let mut a: HashSet<Rc<Job>> = HashSet::new();
    // a.remove(job);

    // if shell.background_jobs_mut().remove(job) {
    //     // The job was a background job. Notify the user that the job
    //     // has finished.
    //     println!("[{}] Done: {}", job.id, job.cmd);
    // }

    shell.jobs_mut().remove(&job.id).unwrap();

    if let Some(ref last_job) = shell.last_fore_job {
        if job.id == last_job.id {
            shell.last_fore_job = None;
        }
    }
}
