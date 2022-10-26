use crate::builtins::BuiltinCommandError;
use crate::expand::{expand_word_into_string, expand_words};
use crate::parser::{self, Ast, Initializer, RunIf, Term};
use crate::process::{
    run_external_command, run_in_foreground, run_internal_command, wait_child, wait_for_job,
    Context, ProcessState,
};
use crate::resolve::resolve_alias;
use crate::shell::Shell;
use crate::variable::Value;
use crate::ExitStatus;

use nix::unistd::{close, fork, pipe, setpgid, ForkResult, Pid};
use std::os::unix::io::RawFd;
use tracing::debug;

pub fn eval(
    shell: &mut Shell,
    ast: &Ast,
    stdin: RawFd,
    stdout: RawFd,
    stderr: RawFd,
) -> ExitStatus {
    debug!("ast: {:#?}", ast);
    run_terms(shell, &ast.terms, stdin, stdout, stderr)
}

pub fn run_terms(
    shell: &mut Shell,
    terms: &[Term],
    stdin: RawFd,
    stdout: RawFd,
    stderr: RawFd,
) -> ExitStatus {
    let mut last_status = ExitStatus::ExitedWith(0);
    for term in terms {
        for pipeline in &term.pipelines {
            match (last_status, &pipeline.run_if) {
                (ExitStatus::ExitedWith(0), RunIf::Success) => (),
                (ExitStatus::ExitedWith(_), RunIf::Failure) => (),
                (_, RunIf::Always) => (),
                _ => continue,
            }

            last_status = run_pipeline(
                shell,
                &term.code,
                pipeline,
                stdin,
                stdout,
                stderr,
                term.background,
            );
        }
    }

    last_status
}

/// Runs commands in a subshell (`$()` or `<()`).
pub fn eval_in_subshell(shell: &mut Shell, terms: &[parser::Term]) -> anyhow::Result<(i32, i32)> {
    let (pipe_out, pipe_in) = pipe().expect("failed to create a pipe");

    let ctx = Context {
        stdin: 0,
        stdout: pipe_in,
        stderr: 2,
        pgid: None,
        background: false,
        interactive: false,
    };

    let pid = spawn_subshell(shell, terms, &ctx)?;
    close(pipe_in).ok();
    let status = wait_child(pid).unwrap_or(1);
    Ok((status, pipe_out))
}

fn spawn_subshell(shell: &mut Shell, terms: &[parser::Term], ctx: &Context) -> anyhow::Result<Pid> {
    match unsafe { fork() }.expect("failed to fork") {
        ForkResult::Parent { child } => Ok(child),
        ForkResult::Child => {
            let status = match run_terms(shell, terms, ctx.stdin, ctx.stdout, ctx.stderr) {
                ExitStatus::ExitedWith(status) => status,
                _ => 1,
            };

            std::process::exit(status);
        }
    }
}

fn run_pipeline(
    shell: &mut Shell,
    code: &str,
    pipeline: &parser::Pipeline,
    pipeline_stdin: RawFd,
    pipeline_stdout: RawFd,
    stderr: RawFd,
    background: bool,
) -> ExitStatus {
    // Invoke commands in a pipeline.
    let mut last_result = None;
    let mut iter = pipeline.commands.iter().peekable();
    let mut childs = Vec::new();
    let mut stdin = pipeline_stdin;
    let mut pgid = None;
    while let Some(command) = iter.next() {
        let stdout;
        let pipes = if iter.peek().is_some() {
            // There is a next command in the pipeline (e.g. date in
            // `date | hexdump`). Create and connect a pipe.
            let (pipe_out, pipe_in) = pipe().expect("failed to create a pipe");
            stdout = pipe_in;
            Some((pipe_out, pipe_in))
        } else {
            // The last command in the pipeline.
            stdout = pipeline_stdout;
            None
        };

        let result = run_command(
            shell,
            command,
            &Context {
                stdin,
                stdout,
                stderr,
                pgid,
                background,
                interactive: shell.interactive(),
            },
        );

        if let Some((pipe_out, pipe_in)) = pipes {
            stdin = pipe_out;
            // `pipe_in` is used by a child process and is no longer needed.
            close(pipe_in).expect("failed to close pipe_in");
        }

        last_result = match result {
            Ok(ExitStatus::Running(pid)) => {
                if pgid.is_none() {
                    // The first child (the process group leader) pid is used for pgid.
                    pgid = Some(pid);
                }

                if shell.interactive {
                    setpgid(pid, pgid.unwrap()).expect("failed to setpgid");
                }

                childs.push(pid);
                Some(ExitStatus::Running(pid))
            }
            Ok(ExitStatus::ExitedWith(status)) => Some(ExitStatus::ExitedWith(status)),
            Err(err) => {
                unimplemented!("error: {}", err);
            }
        };
    }

    // Wait for the last command in the pipeline.
    match last_result {
        Some(ExitStatus::ExitedWith(status)) => {
            shell.set_last_status(status);
            ExitStatus::ExitedWith(status)
        }
        Some(ExitStatus::Running(_)) => {
            let cmd_name = code.to_owned();
            let job = shell.create_job(cmd_name, pgid.unwrap(), childs);

            if !shell.interactive {
                match wait_for_job(shell, &job) {
                    ProcessState::Completed(status) => {
                        shell.set_last_status(status);
                        ExitStatus::ExitedWith(status)
                    }
                    ProcessState::Stopped(_) => ExitStatus::Running(pgid.unwrap()),
                    _ => unreachable!(),
                }
            } else {
                match run_in_foreground(shell, &job) {
                    ProcessState::Completed(status) => ExitStatus::ExitedWith(status),
                    ProcessState::Stopped(_) => ExitStatus::Running(pgid.unwrap()),
                    _ => unreachable!(),
                }
            }
        }
        None => {
            debug!("nothing to execute");
            ExitStatus::ExitedWith(0)
        }
    }
}

fn run_command(
    shell: &mut Shell,
    command: &parser::Command,
    ctx: &Context,
) -> anyhow::Result<ExitStatus> {
    debug!("run_command: {:?}", command);
    let result = match command {
        parser::Command::SimpleCommand {
            argv,
            redirects,
            assignments,
        } => run_simple_command(shell, ctx, argv, redirects, assignments)?,
        _ => unimplemented!("command: {:?}", command),
    };

    Ok(result)
}

fn run_simple_command(
    shell: &mut Shell,
    ctx: &Context,
    argv: &[parser::Word],
    redirects: &[parser::Redirection],
    assignments: &[parser::Assignment],
) -> anyhow::Result<ExitStatus> {
    let argv = expand_words(shell, &resolve_alias(shell, argv))?;
    if argv.is_empty() {
        return Ok(ExitStatus::ExitedWith(0));
    }

    // TODO: support functions

    // Internal commands
    let result = run_internal_command(shell, &argv, ctx.stdin, ctx.stdout, ctx.stderr, redirects);
    match result {
        Ok(status) => return Ok(status),
        Err(err) => match err.downcast_ref::<BuiltinCommandError>() {
            Some(BuiltinCommandError::NotFound) => (),
            _ => return Err(err),
        },
    }

    debug!("argv: {:?}", argv);
    // External commands
    run_external_command(shell, ctx, argv, redirects, assignments)
}

pub fn evaluate_initializer(shell: &mut Shell, initializer: &Initializer) -> anyhow::Result<Value> {
    match initializer {
        Initializer::String(ref word) => Ok(Value::String(expand_word_into_string(shell, word)?)),
        Initializer::Array(ref words) => {
            let elems = expand_words(shell, words)?;
            match (elems.len(), elems.get(0)) {
                (1, Some(body)) if body.is_empty() => {
                    // Make `foo=()' an empty array.
                    Ok(Value::Array(vec![]))
                }
                _ => Ok(Value::Array(elems)),
            }
        }
    }
}
