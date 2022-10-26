use super::{BuiltinCommand, BuiltinCommandContext};
use crate::ExitStatus;

use std::io::Write;
use std::path::Path;
use tracing::debug;

pub struct Cd;

impl BuiltinCommand for Cd {
    fn run(&self, ctx: &mut BuiltinCommandContext) -> ExitStatus {
        debug!("cd: argv={:?}", ctx.argv);
        let current_dir = std::env::current_dir().expect("failed to getcwd()");
        let (dir, pushd) = match ctx.argv.get(1).map(|s| s.as_str()) {
            Some("-") => {
                if let Some(d) = ctx.shell.popd() {
                    (d, false)
                } else {
                    return ExitStatus::ExitedWith(1);
                }
            }
            Some(dir) if dir.starts_with('/') => (dir.to_string(), true),
            // relative path
            Some(dir) => (
                Path::new(&current_dir)
                    .join(dir)
                    .to_string_lossy()
                    .into_owned(),
                true,
            ),
            // with no arguments
            None => (
                if let Some(home_dir) = dirs::home_dir() {
                    home_dir.to_string_lossy().into_owned()
                } else {
                    String::from("/")
                },
                true,
            ),
        };

        if pushd {
            ctx.shell.pushd(current_dir.to_str().unwrap().to_owned());
        }

        match std::env::set_current_dir(&dir) {
            Ok(_) => ExitStatus::ExitedWith(0),
            Err(err) => {
                writeln!(ctx.stderr, "smash: cd: {}: `{}'", err, dir).ok();
                ExitStatus::ExitedWith(1)
            }
        }
    }
}
