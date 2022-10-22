use crate::eval::eval;
use crate::parser;
use crate::path::PathTable;
use crate::variable::Value;
use crate::ExitStatus;
use std::os::unix::io::RawFd;
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

    pub fn set(&mut self, key: &str, value: Value, is_local: bool) {
        // let frame = if is_local {
        //     self.current_frame_mut()
        // } else {
        //     &mut self.global
        // };

        // frame.set(key, value.clone());

        if !is_local && key == "PATH" {
            // $PATH is being updated. Reload directories.
            if let Value::String(ref path) = value {
                self.path_table.scan(path);
            }
        }
    }
}
