use crate::shell::Shell;
use crate::ExitStatus;

use thiserror::Error;

mod cd;
mod exit;
mod export;

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
        "exit" => Some(Box::new(exit::Exit)),
        "export" => Some(Box::new(export::Export)),
        _ => None,
    }
}