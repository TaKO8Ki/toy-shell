use super::{BuiltinCommand, BuiltinCommandContext};
use crate::ExitStatus;

pub struct Exit;

impl BuiltinCommand for Exit {
    fn run(&self, _: &mut BuiltinCommandContext) -> ExitStatus {
        std::process::exit(0);
    }
}
