use crate::parser::ExpansionOp;
use crate::parser::Word;
use crate::parser::{self, Span};
use crate::shell::Shell;

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
                // Internally used by the parser.
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
    op: &ExpansionOp,
) -> anyhow::Result<Vec<Option<String>>> {
    match name {
        "?" => {
            return Ok(vec![Some(shell.last_status().to_string())]);
        }
        // "!" => {
        //     let pgid = match shell.last_back_job() {
        //         Some(job) => job.pgid.to_string(),
        //         None => 0.to_string(),
        //     };

        //     return Ok(vec![Some(pgid)]);
        // }
        // "0" => {
        //     return Ok(vec![Some(shell.script_name.clone())]);
        // }
        // "$" => {
        //     return Ok(vec![Some(shell.shell_pgid.to_string())]);
        // }
        // "#" => {
        //     return Ok(vec![Some(shell.current_frame().num_args().to_string())]);
        // }
        // "*" => {
        //     let args = shell.current_frame().get_string_args();
        //     let expanded = args.join(" ");
        //     return Ok(vec![Some(expanded)]);
        // }
        // "@" => {
        //     let args = shell.current_frame().get_string_args();
        //     return Ok(args.iter().map(|a| Some(a.to_owned())).collect());
        // }
        _ => {
            debug!("aaaaaaaaaaaaaaaa: {:?}, {:?}", name, shell.get(name));
            if let Some(var) = shell.get(name) {
                // $<name> is defined.
                let value = var.value();
                match (op, value) {
                    // (ExpansionOp::Length, Some(_)) => {
                    //     return Ok(vec![Some(var.as_str().len().to_string())]);
                    // }
                    // (ExpansionOp::Length, None) => {
                    //     return Ok(vec![Some(0.to_string())]);
                    // }
                    // (ExpansionOp::GetNullableOrDefaultAndAssign(_), None) => {
                    //     return Ok(vec![None]);
                    // }
                    // (ExpansionOp::GetNullableOrDefault(_), None) => {
                    //     return Ok(vec![None]);
                    // }
                    // (
                    //     ExpansionOp::Subst {
                    //         pattern,
                    //         replacement,
                    //         replace_all,
                    //     },
                    //     Some(_),
                    // ) => {
                    //     let content = var.as_str().to_string();
                    //     let replaced =
                    //         replace_pattern(shell, pattern, &content, replacement, *replace_all)?;
                    //     return Ok(vec![Some(replaced)]);
                    // }
                    (_, _) => {
                        return Ok(vec![Some(var.as_str().to_string())]);
                    }
                };
            }
        }
    }

    smash_err!("undefined variable `{}`", name);
    std::process::exit(1);

    // The variable is not defined or is nulll
    // http://pubs.opengroup.org/onlinepubs/009695399/utilities/xcu_chap02.html#tag_02_06_02
    // match op {
    //     ExpansionOp::Length => {
    //         if shell.nounset {
    //             print_err!("undefined variable `{}'", name);
    //             std::process::exit(1);
    //         }

    //         Ok(vec![Some("0".to_owned())])
    //     }
    //     ExpansionOp::GetOrEmpty => {
    //         if shell.nounset {
    //             print_err!("undefined variable `{}'", name);
    //             std::process::exit(1);
    //         }

    //         Ok(vec![Some("".to_owned())])
    //     }
    //     ExpansionOp::GetOrDefault(word) | ExpansionOp::GetNullableOrDefault(word) => {
    //         expand_word_into_string(shell, word).map(|s| vec![Some(s)])
    //     }
    //     ExpansionOp::GetOrDefaultAndAssign(word)
    //     | ExpansionOp::GetNullableOrDefaultAndAssign(word) => {
    //         let content = expand_word_into_string(shell, word)?;
    //         shell.set(name, Value::String(content.clone()), false);
    //         Ok(vec![Some(content)])
    //     }
    //     ExpansionOp::Subst { .. } => Ok(vec![Some("".to_owned())]),
    // }
}
