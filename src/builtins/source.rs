use super::{BuiltinCommand, BuiltinCommandContext};
use crate::ExitStatus;

use std::io::Write;

pub struct Source;

impl BuiltinCommand for Source {
    fn run(&self, ctx: &mut BuiltinCommandContext) -> ExitStatus {
        if let Some(filepath) = ctx.argv.get(1) {
            match ctx.shell.run_file(std::path::PathBuf::from(&filepath)) {
                Ok(status) => status,
                Err(err) => {
                    smash_err!("smash: failed open the file: {:?}", err);
                    ExitStatus::ExitedWith(1)
                }
            }
        } else {
            writeln!(ctx.stderr, "nsh: source: filename argument required").ok();
            ctx.stderr.flush().ok();
            ExitStatus::ExitedWith(0)
        }
    }
}
