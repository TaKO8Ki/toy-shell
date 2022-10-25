use crate::shell::Shell;
use crate::ExitStatus;

use thiserror::Error;

mod cd;
mod echo;
mod eval;
mod exit;
mod export;
mod source;

pub trait BuiltinCommand {
    fn run(&self, ctx: BuiltinCommandContext) -> ExitStatus;
}

pub struct BuiltinCommandContext<'a> {
    pub argv: &'a [String],
    pub shell: &'a mut Shell,
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
        _ => None,
    }
}
