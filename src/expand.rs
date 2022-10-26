use crate::eval::eval_in_subshell;
use crate::parser::ExpansionOp;
use crate::parser::Span;
use crate::parser::Word;
use crate::shell::Shell;

use std::fs::File;
use std::io::Read;
use std::os::unix::io::FromRawFd;
use tracing::debug;

pub fn expand_words(shell: &mut Shell, words: &[Word]) -> anyhow::Result<Vec<String>> {
    debug!("expand_words: {:?}", words);
    let mut evaluated = Vec::new();
    for word in words {
        let mut ws = Vec::new();
        for w in expand_word_into_vec(shell, word, &shell.ifs())? {
            debug!("w: {:?}", w);
            ws.push(w);
        }

        evaluated.extend(ws);
    }

    debug!("expand_words: {:?}", evaluated);
    Ok(evaluated)
}

pub fn expand_word_into_string(shell: &mut Shell, word: &Word) -> anyhow::Result<String> {
    let ws: Vec<String> = expand_word_into_vec(shell, word, &shell.ifs())?;
    Ok(ws.join(""))
}

pub fn expand_word_into_vec(
    shell: &mut Shell,
    word: &Word,
    ifs: &str,
) -> anyhow::Result<Vec<String>> {
    let mut words = Vec::new();
    let mut current_word = Vec::new();
    for span in word.spans() {
        let (frags, expand) = match span {
            Span::LiteralChars(..) => {
                unreachable!()
            }
            Span::Literal(s) => (vec![s.clone()], false),
            Span::Parameter { name, op, quoted } => {
                let mut frags = Vec::new();
                for value in expand_param(shell, name, op)? {
                    let frag = value.unwrap_or_else(|| "".to_owned());
                    frags.push(frag);
                }
                (frags, !quoted)
            }
            Span::Tilde(_) => {
                let dir = dirs::home_dir().unwrap().to_str().unwrap().to_owned();
                (vec![dir], false)
            }
            Span::Command { body, quoted } => {
                let (_, stdout) = eval_in_subshell(shell, body)?;

                let mut raw_stdout = Vec::new();
                unsafe { File::from_raw_fd(stdout).read_to_end(&mut raw_stdout).ok() };

                let output = std::str::from_utf8(&raw_stdout)
                    .map_err(|err| {
                        smash_err!("binary in variable/expansion is not supported");
                        err
                    })?
                    .trim_end_matches('\n')
                    .to_owned();

                (vec![output], !quoted)
            }
        };

        let frags_len = frags.len();
        for frag in frags {
            if expand {
                if !current_word.is_empty() {
                    words.push(current_word.into_iter().collect::<String>());
                    current_word = Vec::new();
                }

                for word in frag.split(|c| ifs.contains(c)) {
                    words.push(word.to_string());
                }
            } else {
                current_word.push(frag);
            }

            if frags_len > 1 && !current_word.is_empty() {
                words.push(current_word.into_iter().collect::<String>());
                current_word = Vec::new();
            }
        }
    }

    if !current_word.is_empty() {
        words.push(current_word.into_iter().collect::<String>());
    }

    if words.is_empty() {
        Ok(vec![String::new()])
    } else {
        Ok(words)
    }
}

pub fn expand_param(
    shell: &mut Shell,
    name: &str,
    _op: &ExpansionOp,
) -> anyhow::Result<Vec<Option<String>>> {
    match name {
        "?" => {
            return Ok(vec![Some(shell.last_status().to_string())]);
        }
        // TODO: support the other expansion ops
        _ => {
            debug!("{:?}={:?}", name, shell.get(name));
            if let Some(var) = shell.get(name) {
                return Ok(vec![Some(var.as_str().to_string())]);
            }
        }
    }

    smash_err!("undefined variable `{}`", name);
    std::process::exit(1);

    // TODO: handling errors depending on the expansion op
}
