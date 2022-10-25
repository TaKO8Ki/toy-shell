use crate::builtins::BuiltinCommandError;
use crate::expand::expand_words;
use crate::parser;
use crate::parser::{Ast, RunIf, Term};
use crate::process::{
    run_external_command, run_in_foreground, run_internal_command, wait_child, wait_for_job,
    Context, ProcessState,
};
use crate::shell::Shell;
use crate::ExitStatus;

use nix::unistd::{close, execv, fork, getpid, pipe, setpgid, ForkResult, Pid};
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
            // Should we execute the pipline?
            match (last_status, &pipeline.run_if) {
                (ExitStatus::ExitedWith(0), RunIf::Success) => (),
                (ExitStatus::ExitedWith(_), RunIf::Failure) => (),
                (ExitStatus::Break, _) => return ExitStatus::Break,
                (ExitStatus::Continue, _) => return ExitStatus::Continue,
                (ExitStatus::Return, _) => return ExitStatus::Return,
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
            Ok(ExitStatus::Break) => {
                last_result = Some(ExitStatus::Break);
                break;
            }
            Ok(ExitStatus::Continue) => {
                last_result = Some(ExitStatus::Continue);
                break;
            }
            Ok(ExitStatus::Return) => {
                last_result = Some(ExitStatus::Return);
                break;
            }
            Ok(ExitStatus::NoExec) => {
                last_result = Some(ExitStatus::NoExec);
                break;
            }
            Err(err) => {
                // if err
                //     .find_root_cause()
                //     .downcast_ref::<NoMatchesError>()
                //     .is_some()
                // {
                //     debug!("error: no matches");
                //     last_result = Some(ExitStatus::ExitedWith(1));
                //     break;
                // }

                unreachable!();
            }
        };
    }

    // Wait for the last command in the pipeline.
    let last_status = match last_result {
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
                match run_in_foreground(shell, &job, false) {
                    ProcessState::Completed(status) => ExitStatus::ExitedWith(status),
                    ProcessState::Stopped(_) => ExitStatus::Running(pgid.unwrap()),
                    _ => unreachable!(),
                }
            }
            // } else if background {
            //     // run_in_background(shell, &job, false);
            //     ExitStatus::Running(pgid.unwrap())
            // } else {
            //     match run_in_foreground(shell, &job, false) {
            //         ProcessState::Completed(status) => ExitStatus::ExitedWith(status),
            //         ProcessState::Stopped(_) => ExitStatus::Running(pgid.unwrap()),
            //         _ => unreachable!(),
            //     }
            // }
        }
        Some(ExitStatus::Break) => {
            return ExitStatus::Break;
        }
        Some(ExitStatus::Continue) => {
            return ExitStatus::Continue;
        }
        Some(ExitStatus::Return) => {
            return ExitStatus::Return;
        }
        Some(ExitStatus::NoExec) => {
            return ExitStatus::NoExec;
        }
        None => {
            debug!("nothing to execute");
            ExitStatus::ExitedWith(0)
        }
    };

    if shell.errexit {
        if let ExitStatus::ExitedWith(status) = last_status {
            if status != 0 {
                std::process::exit(status);
            }
        }
    }

    last_status
}

fn run_command(
    shell: &mut Shell,
    command: &parser::Command,
    ctx: &Context,
) -> anyhow::Result<ExitStatus> {
    if shell.noexec {
        return Ok(ExitStatus::NoExec);
    }

    debug!("run_command: {:?}", command);
    let result = match command {
        parser::Command::SimpleCommand {
            argv,
            redirects,
            assignments,
        } => run_simple_command(shell, ctx, argv, redirects, assignments)?,
        // parser::Command::If {
        //     condition,
        //     then_part,
        //     elif_parts,
        //     else_part,
        //     redirects,
        // } => run_if_command(
        //     shell, ctx, condition, then_part, elif_parts, else_part, redirects,
        // )?,
        // parser::Command::While { condition, body } => {
        //     run_while_command(shell, ctx, condition, body)?
        // }
        // parser::Command::Case { word, cases } => run_case_command(shell, ctx, word, cases)?,
        // parser::Command::For {
        //     var_name,
        //     words,
        //     body,
        // } => run_for_command(shell, ctx, var_name, words, body)?,
        // parser::Command::ArithFor {
        //     init,
        //     cond,
        //     update,
        //     body,
        // } => run_arith_for_command(shell, ctx, init, cond, update, body)?,
        // parser::Command::LocalDef { declarations } => run_local_command(shell, declarations)?,
        // parser::Command::FunctionDef { name, body } => {
        //     shell.set(name, Value::Function(body.clone()), true);
        //     ExitStatus::ExitedWith(0)
        // }
        // parser::Command::Assignment { assignments } => {
        //     for assign in assignments {
        //         let value = evaluate_initializer(shell, &assign.initializer)?;
        //         shell.assign(&assign.name, value)
        //     }
        //     ExitStatus::ExitedWith(0)
        // }
        // parser::Command::Cond(expr) => {
        //     let result = evaluate_cond(shell, expr)?;
        //     if result {
        //         ExitStatus::ExitedWith(0)
        //     } else {
        //         ExitStatus::ExitedWith(1)
        //     }
        // }
        // parser::Command::Group { terms } => {
        //     run_terms(shell, terms, ctx.stdin, ctx.stdout, ctx.stderr)
        // }
        // parser::Command::SubShellGroup { terms } => {
        //     let pid = spawn_subshell(shell, terms, ctx)?;
        //     let status = wait_child(pid).unwrap_or(1);
        //     ExitStatus::ExitedWith(status)
        // }
        // parser::Command::Return { status } => {
        //     if let Some(status) = status {
        //         shell.set_last_status(*status);
        //     }

        //     ExitStatus::Return
        // }
        // parser::Command::Break => ExitStatus::Break,
        // parser::Command::Continue => ExitStatus::Continue,
        _ => todo!("unexpected commands"),
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
    // let argv = expand_words(shell, &expand_alias(shell, argv))?;
    let argv = expand_words(shell, argv)?;
    if argv.is_empty() {
        // `argv` is empty. For example bash accepts `> foo.txt`; it creates an empty file
        // named "foo.txt".
        return Ok(ExitStatus::ExitedWith(0));
    }

    // Functions
    // let argv0 = argv[0].as_str();
    // if let Some(var) = shell.get(argv0) {
    //     if var.is_function() {
    //         let args: Vec<String> = argv.iter().skip(1).cloned().collect();
    //         return call_function(shell, argv0, ctx, &args, vec![]);
    //     }
    // }

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
