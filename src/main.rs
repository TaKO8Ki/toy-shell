use crossterm::terminal;
use crossterm::tty::IsTty;
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
mod parser;
mod path;
mod process;
mod shell;
mod variable;

fn main() {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    let mut shell = Shell::new();

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
