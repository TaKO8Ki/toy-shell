use crossterm::terminal;
use crossterm::tty::IsTty;
use std::fs::File;
use std::path::Path;
use tracing_subscriber::{self, fmt, prelude::*, EnvFilter};

use event::SmashState;
use process::ExitStatus;
use shell::Shell;
use variable::Value;

#[macro_use]
mod macros;

mod builtins;
mod context_parser;
mod eval;
mod event;
mod expand;
mod highlight;
mod history;
mod parser;
mod path;
mod process;
mod resolve;
mod shell;
mod variable;

fn main() {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    let home_dir = dirs::home_dir().expect("failed to get the path to the home directory");
    let history_path = Path::new(&home_dir).join(".smash_history");
    if !history_path.exists() {
        File::create(&history_path).unwrap();
    }

    let mut shell = Shell::new(&history_path);

    for (key, value) in std::env::vars() {
        shell.set(&key, Value::String(value.to_owned()), false);
    }

    let home_dir = dirs::home_dir().unwrap();
    shell.run_file(home_dir.join(".smashrc")).ok();

    let is_tty = std::io::stdout().is_tty();
    shell.set_interactive(is_tty);

    SmashState::new(shell).run();

    terminal::disable_raw_mode().unwrap();
}
