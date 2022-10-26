use super::{BuiltinCommand, BuiltinCommandContext};
use crate::ExitStatus;

pub struct Eval;

impl BuiltinCommand for Eval {
    fn run(&self, ctx: &mut BuiltinCommandContext) -> ExitStatus {
        let mut program = String::new();
        for arg in ctx.argv.iter().skip(1) {
            program += arg;
            program.push(' ');
        }

        ctx.shell.run_script(&program)
    }
}
