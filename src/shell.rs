use crate::eval::eval;
use crate::parser;
use crate::path::PathTable;
use crate::process::{Job, JobId, ProcessState};
use crate::variable::{Frame, Value, Variable};
use crate::ExitStatus;

use nix::sys::termios::{tcgetattr, Termios};
use nix::unistd::{getpid, Pid};
use std::collections::{HashMap, HashSet};
use std::os::unix::io::RawFd;
use std::rc::Rc;
use tracing::debug;

pub struct Shell {
    /// `set -e`
    pub errexit: bool,
    /// `set -u`
    pub nounset: bool,
    /// `set -n`
    pub noexec: bool,

    last_status: i32,

    pub interactive: bool,
    path_table: PathTable,
    pub last_fore_job: Option<Rc<Job>>,
    background_jobs: HashSet<Rc<Job>>,
    states: HashMap<Pid, ProcessState>,
    pub shell_pgid: Pid,
    pub shell_termios: Option<Termios>,
    pid_job_mapping: HashMap<Pid, Rc<Job>>,
    jobs: HashMap<JobId, Rc<Job>>,
    cd_stack: Vec<String>,

    /// Local scopes (variables declared with `local').
    frames: Vec<Frame>,
    /// Global scope.
    global: Frame,

    exported: HashSet<String>,
}

impl Shell {
    pub fn new() -> Self {
        Self {
            last_status: 0,
            errexit: false,
            nounset: false,
            noexec: false,
            interactive: false,
            path_table: PathTable::new(),
            last_fore_job: None,
            background_jobs: HashSet::new(),
            states: HashMap::new(),
            shell_pgid: getpid(),
            shell_termios: None,
            pid_job_mapping: HashMap::new(),
            jobs: HashMap::new(),
            cd_stack: Vec::new(),
            frames: Vec::new(),
            global: Frame::new(),
            exported: HashSet::new(),
        }
    }

    #[inline]
    pub fn interactive(&self) -> bool {
        self.interactive
    }

    pub fn path_table(&self) -> &PathTable {
        &self.path_table
    }

    /// Parses and runs a script. Stdin/stdout/stderr are 0, 1, 2, respectively.
    pub fn run_str(&mut self, script: &str) -> ExitStatus {
        // Inherit shell's stdin/stdout/stderr.
        let stdin = 0;
        let stdout = 1;
        let stderr = 2;
        self.run_str_with_stdio(script, stdin, stdout, stderr)
    }

    /// Parses and runs a script in the given context.
    pub fn run_str_with_stdio(
        &mut self,
        script: &str,
        stdin: RawFd,
        stdout: RawFd,
        stderr: RawFd,
    ) -> ExitStatus {
        match parser::parse(script) {
            Ok(ast) => eval(self, &ast, stdin, stdout, stderr),
            Err(parser::ParseError::Empty) => {
                // Just ignore.
                ExitStatus::ExitedWith(0)
            }
            Err(parser::ParseError::Fatal(err)) => {
                debug!("parse error: {}", err);
                ExitStatus::ExitedWith(-1)
            }
        }
    }

    pub fn set_last_status(&mut self, status: i32) {
        self.last_status = status;
    }

    pub fn last_status(&self) -> i32 {
        self.last_status
    }

    pub fn ifs(&self) -> String {
        self.get_str("IFS").unwrap_or_else(|| "\n\t ".to_owned())
    }

    pub fn get_str(&self, key: &str) -> Option<String> {
        match self.get(key) {
            Some(var) => match var.value() {
                Some(Value::String(ref s)) => Some(s.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn set(&mut self, key: &str, value: Value, is_local: bool) {
        let frame = if is_local {
            self.current_frame_mut()
        } else {
            &mut self.global
        };

        frame.set(key, value.clone());

        if !is_local && key == "PATH" {
            // $PATH is being updated. Reload directories.
            if let Value::String(ref path) = value {
                self.path_table.scan(path);
            }
        }
    }

    pub fn background_jobs_mut(&mut self) -> &mut HashSet<Rc<Job>> {
        &mut self.background_jobs
    }

    pub fn get_process_state(&self, pid: Pid) -> Option<&ProcessState> {
        self.states.get(&pid)
    }

    pub fn set_process_state(&mut self, pid: Pid, state: ProcessState) {
        self.states.insert(pid, state);
    }

    pub fn create_job(&mut self, name: String, pgid: Pid, childs: Vec<Pid>) -> Rc<Job> {
        let id = self.alloc_job_id();
        let job = Rc::new(Job::new(id, pgid, name, childs.clone()));
        for child in childs {
            self.set_process_state(child, ProcessState::Running);
            self.pid_job_mapping.insert(child, job.clone());
        }

        self.jobs_mut().insert(id, job.clone());
        job
    }

    pub fn jobs_mut(&mut self) -> &mut HashMap<JobId, Rc<Job>> {
        &mut self.jobs
    }

    fn alloc_job_id(&mut self) -> JobId {
        let mut id = 1;
        while self.jobs.contains_key(&JobId::new(id)) {
            id += 1;
        }

        JobId::new(id)
    }

    pub fn set_interactive(&mut self, interactive: bool) {
        self.interactive = interactive;
        self.shell_termios = if interactive {
            Some(tcgetattr(0 /* stdin */).expect("failed to tcgetattr"))
        } else {
            None
        };
    }

    pub fn pushd(&mut self, path: String) {
        self.cd_stack.push(path);
    }

    pub fn popd(&mut self) -> Option<String> {
        self.cd_stack.pop()
    }

    pub fn get(&self, key: &str) -> Option<Rc<Variable>> {
        if let Some(var) = self.current_frame().get(key) {
            Some(var)
        } else {
            self.global.get(key)
        }
    }

    #[inline]
    pub fn current_frame(&self) -> &Frame {
        self.frames.last().unwrap_or(&self.global)
    }

    #[inline]
    pub fn enter_frame(&mut self) {
        self.frames.push(Frame::new());
    }

    #[inline]
    pub fn leave_frame(&mut self) {
        self.frames.pop();
    }

    #[inline]
    pub fn in_global_frame(&self) -> bool {
        self.frames.is_empty()
    }

    #[inline]
    pub fn current_frame_mut(&mut self) -> &mut Frame {
        self.frames.last_mut().unwrap_or(&mut self.global)
    }

    pub fn exported_names(&self) -> std::collections::hash_set::Iter<String> {
        self.exported.iter()
    }

    pub fn export(&mut self, name: &str) {
        self.exported.insert(name.to_string());
    }
}
