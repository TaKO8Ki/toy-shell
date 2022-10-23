use crossterm::event::Event as TermEvent;
use crossterm::terminal::{self, enable_raw_mode};
use nix::sys::signal::{sigaction, SaFlags, SigAction, SigHandler, SigSet, Signal};

use crossterm::tty::IsTty;
use nix::unistd::Pid;
use signal_hook::{self, iterator::Signals};
use std::os::unix::io::RawFd;
use std::sync::mpsc;
use std::time::Duration;
use tracing::debug;
use tracing_subscriber;

use event::SmashState;
use process::ExitStatus;
use shell::Shell;
use variable::Value;

#[macro_use]
mod macros;

mod builtins;
mod eval;
mod event;
mod expand;
mod parser;
mod path;
mod process;
mod shell;
mod variable;

/// The process execution environment.
#[derive(Debug, Copy, Clone)]
pub struct Context {
    pub stdin: RawFd,
    pub stdout: RawFd,
    pub stderr: RawFd,
    pub pgid: Option<Pid>,
    /// The process should be executed in background.
    pub background: bool,
    /// Is the shell interactive?
    pub interactive: bool,
}

fn main() {
    tracing_subscriber::fmt::init();

    let mut shell = Shell::new();

    for (key, value) in std::env::vars() {}

    let (tx, rx) = mpsc::channel();
    let tx2 = tx.clone();
    std::thread::spawn(move || {
        let signals = Signals::new(&[signal_hook::SIGWINCH]).unwrap();
        for signal in signals {
            match signal {
                signal_hook::SIGWINCH => {
                    tx2.send(event::Event::ScreenResized).ok();
                }
                _ => {
                    tracing::warn!("unhandled signal: {}", signal);
                }
            }
        }

        unreachable!();
    });

    for (key, value) in std::env::vars() {
        shell.set(&key, Value::String(value.to_owned()), false);
    }

    // Try executing ~/.smashrc
    let home_dir = dirs::home_dir().unwrap();
    shell.run_file(home_dir.join(".smashrc")).ok();

    let is_tty = std::io::stdout().is_tty();
    shell.set_interactive(is_tty);

    let mut state = SmashState::new(shell);
    enable_raw_mode().ok();
    state.render_prompt();

    let action = SigAction::new(SigHandler::SigIgn, SaFlags::empty(), SigSet::empty());
    unsafe {
        sigaction(Signal::SIGINT, &action).expect("failed to sigaction");
        sigaction(Signal::SIGQUIT, &action).expect("failed to sigaction");
        sigaction(Signal::SIGTSTP, &action).expect("failed to sigaction");
        sigaction(Signal::SIGTTIN, &action).expect("failed to sigaction");
        sigaction(Signal::SIGTTOU, &action).expect("failed to sigaction");
    }

    loop {
        let mut started_at = None;

        match crossterm::event::poll(Duration::from_millis(100)) {
            Ok(true) => loop {
                if let Ok(TermEvent::Key(ev)) = crossterm::event::read() {
                    state.handle_key_event(&ev);
                }

                match crossterm::event::poll(Duration::from_millis(0)) {
                    Ok(true) => (), // Continue reading stdin.
                    _ => break,
                }
            },
            _ => {
                if let Ok(_) = rx.try_recv() {
                    started_at = Some(std::time::SystemTime::now());
                    // self.handle_event(ev);
                }
            }
        }
    }

    // execute!(stdout, terminal::LeaveAlternateScreen).unwrap();

    terminal::disable_raw_mode().unwrap();
}
