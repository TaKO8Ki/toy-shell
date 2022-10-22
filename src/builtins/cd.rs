use super::BuiltinCommand;
use crate::ExitStatus;

pub struct Cd;

impl BuiltinCommand for Cd {
    fn run(&self) -> ExitStatus {
        todo!()
    }
}
