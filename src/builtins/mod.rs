use thiserror::Error;

mod cd;
mod exit;

use crate::ExitStatus;

pub trait BuiltinCommand {
    fn run(&self) -> ExitStatus;
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
        _ => None,
    }
}
