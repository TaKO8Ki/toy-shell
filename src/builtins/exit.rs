use super::BuiltinCommand;
use crate::ExitStatus;

pub struct Exit;

impl BuiltinCommand for Exit {
    fn run(&self) -> ExitStatus {
        std::process::exit(0);
    }
}
