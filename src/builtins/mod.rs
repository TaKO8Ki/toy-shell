use crate::fd_file::FdFile;
use crate::shell::Shell;
use crate::ExitStatus;

use thiserror::Error;

mod alias;
mod cd;
mod eval;
mod exit;
mod export;
mod source;

pub trait BuiltinCommand {
    fn run(&self, ctx: &mut BuiltinCommandContext) -> ExitStatus;
}

pub struct BuiltinCommandContext<'a> {
    pub argv: &'a [String],
    pub shell: &'a mut Shell,
    pub stdin: FdFile,
    pub stdout: FdFile,
    pub stderr: FdFile,
}

#[derive(Debug, Error)]
pub enum BuiltinCommandError {
    #[error("command not found")]
    NotFound,
}

pub fn builtin_command(name: &str) -> Option<Box<dyn BuiltinCommand>> {
    match name {
        "cd" => Some(Box::new(cd::Cd)),
        "eval" => Some(Box::new(eval::Eval)),
        "exit" => Some(Box::new(exit::Exit)),
        "export" => Some(Box::new(export::Export)),
        "source" => Some(Box::new(source::Source)),
        "alias" => Some(Box::new(alias::Alias)),
        _ => None,
    }
}
