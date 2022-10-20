use pest::iterators::Pair;
use pest::Parser;
use pest_derive::Parser;
use tracing::debug;

#[derive(Parser)]
#[grammar = "shell.pest"]
struct ShellParser;

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Ast {
    terms: Vec<Term>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Term {
    pub code: String,
    pub pipelines: Vec<Pipeline>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum ParseError {
    Fatal(String),
    Empty,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum RunIf {
    Always,
    /// Run the command if the previous command returned 0.
    Success,
    /// Run the command if the previous command returned non-zero value.
    Failure,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Assignment {
    pub name: String,
    pub initializer: Initializer,
    pub index: Option<Expr>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct BinaryExpr {
    pub lhs: Box<Expr>,
    pub rhs: Box<Expr>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Expr {
    Add(BinaryExpr),
    Sub(BinaryExpr),
    Mul(BinaryExpr),
    Div(BinaryExpr),
    Assign { name: String, rhs: Box<Expr> },
    Literal(i32),

    // `foo` in $((foo + 1))
    Parameter { name: String },

    // Conditions. Evaluated to 1 if it satistifies or 0 if not.
    Eq(Box<Expr>, Box<Expr>),
    Ne(Box<Expr>, Box<Expr>),
    Lt(Box<Expr>, Box<Expr>),
    Le(Box<Expr>, Box<Expr>),
    Gt(Box<Expr>, Box<Expr>),
    Ge(Box<Expr>, Box<Expr>),

    // `i++` and `i--`
    Inc(String),
    Dec(String),

    Expr(Box<Expr>),
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Initializer {
    Array(Vec<Word>),
    String(Word),
}

#[allow(clippy::enum_variant_names)] // Allow SimpleCommand
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Command {
    SimpleCommand {
        argv: Vec<Word>,
        redirects: Vec<Redirection>,
        /// Assignment prefixes. (e.g. "RAILS_ENV=production rails server")
        assignments: Vec<Assignment>,
    },
    // foo=1, bar="Hello World", ...
    Assignment {
        assignments: Vec<Assignment>,
    },
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum LiteralChar {
    Normal(char),
    Escaped(char),
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Redirection {
    pub fd: usize,
    pub direction: RedirectionDirection,
    pub target: RedirectionType,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum RedirectionDirection {
    Input,  // cat < foo.txt or here document
    Output, // cat > foo.txt
    Append, // cat >> foo.txt
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum RedirectionType {
    File(Word),
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Span {
    Literal(String),
    LiteralChars(Vec<LiteralChar>),
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Word(pub Vec<Span>);

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Pipeline {
    pub run_if: RunIf,
    pub commands: Vec<Command>, // Separated by `|'.
}

pub fn parse(script: &str) -> Result<Ast, ParseError> {
    match ShellParser::parse(Rule::script, script) {
        Ok(mut pairs) => {
            let terms = visit_compound_list(pairs.next().unwrap());

            if terms.is_empty() {
                Err(ParseError::Empty)
            } else {
                Ok(Ast { terms })
            }
        }
        Err(err) => Err(ParseError::Fatal(err.to_string())),
    }
}

macro_rules! wsnl {
    ($pairs:expr) => {
        if let Some(next) = $pairs.next() {
            match next.as_rule() {
                Rule::newline => $pairs.next(),
                _ => Some(next),
            }
        } else {
            None
        }
    };
}

fn visit_compound_list(pair: Pair<Rule>) -> Vec<Term> {
    let mut terms = Vec::new();
    let mut inner = pair.into_inner();
    if let Some(and_or_list) = inner.next() {
        let mut background = false;
        let mut rest = None;
        while let Some(sep_or_rest) = wsnl!(inner) {
            debug!(?sep_or_rest);
            match sep_or_rest.as_rule() {
                Rule::compound_list => {
                    rest = Some(sep_or_rest);
                    break;
                }
                _ => {
                    let sep = sep_or_rest.into_inner().next().unwrap();
                    match sep.as_rule() {
                        Rule::background => {
                            background = true;
                        }
                        Rule::newline => {
                            // TODO: handle heredocs
                        }
                        Rule::seq_sep => (),
                        _ => (),
                    }
                }
            }
        }

        if and_or_list.as_rule() == Rule::and_or_list {
            let code = and_or_list.as_str().to_owned().trim().to_owned();
            let pipelines = visit_and_or_list(and_or_list, RunIf::Always);
            terms.push(Term { code, pipelines });
        }

        if let Some(rest) = rest {
            terms.extend(visit_compound_list(rest));
        }
    }

    terms
}

fn visit_and_or_list(pair: Pair<Rule>, run_if: RunIf) -> Vec<Pipeline> {
    let mut terms = Vec::new();
    let mut inner = pair.into_inner();
    if let Some(pipeline) = inner.next() {
        let commands = visit_pipeline(pipeline);
        terms.push(Pipeline { commands, run_if });

        let next_run_if = inner
            .next()
            .map(|sep| match sep.as_span().as_str() {
                "||" => RunIf::Failure,
                "&&" => RunIf::Success,
                _ => RunIf::Always,
            })
            .unwrap_or(RunIf::Always);

        if let Some(rest) = wsnl!(inner) {
            terms.extend(visit_and_or_list(rest, next_run_if));
        }
    }

    terms
}

fn visit_pipeline(pair: Pair<Rule>) -> Vec<Command> {
    let mut commands = Vec::new();
    let mut inner = pair.into_inner();
    while let Some(command) = wsnl!(inner) {
        commands.push(visit_command(command));
    }

    commands
}

fn visit_simple_command(pair: Pair<Rule>) -> Command {
    assert_eq!(pair.as_rule(), Rule::simple_command);

    let mut argv = Vec::new();
    let mut redirects = Vec::new();

    let mut inner = pair.into_inner();
    let assignments_pairs = inner.next().unwrap().into_inner();
    let argv0 = inner.next().unwrap().into_inner().next().unwrap();
    let args = inner.next().unwrap().into_inner();

    argv.push(visit_word(argv0));
    for word_or_redirect in args {
        match word_or_redirect.as_rule() {
            Rule::word => argv.push(visit_word(word_or_redirect)),
            Rule::redirect => redirects.push(visit_redirect(word_or_redirect)),
            _ => unreachable!(),
        }
    }

    let assignments = Vec::new();
    // for assignment in assignments_pairs {
    //     assignments.push(visit_assignment(assignment));
    // }

    Command::SimpleCommand {
        argv,
        redirects,
        assignments,
    }
}

// fn visit_assignment(pair: Pair<Rule>) -> Assignment {
//     let mut inner = pair.into_inner();

//     let name = inner.next().unwrap().as_span().as_str().to_owned();
//     let index = inner
//         .next()
//         .unwrap()
//         .into_inner()
//         .next()
//         .map(|p| visit_expr(p));
//     let initializer = inner.next().unwrap().into_inner().next().unwrap();
//     match initializer.as_rule() {
//         Rule::string_initializer => {
//             let word =
//                 Initializer::String(visit_word(initializer.into_inner().next().unwrap()));
//             Assignment {
//                 name,
//                 initializer: word,
//                 index,
//             }
//         }
//         Rule::array_initializer => {
//             let word = Initializer::Array(
//                 initializer
//                     .into_inner()
//                     .map(|p| visit_word(p))
//                     .collect(),
//             );
//             let index = None;
//             Assignment {
//                 name,
//                 initializer: word,
//                 index,
//             }
//         }
//         _ => unreachable!(),
//     }
// }

// fn visit_expr(pair: Pair<Rule>) -> Expr {
//     let mut inner = pair.clone().into_inner();
//     let first = inner.next().unwrap();
//     let maybe_op = inner.next();

//     match pair.as_rule() {
//         Rule::assign => visit_assign_expr(pair),
//         Rule::arith => visit_arith_expr(pair),
//         Rule::term => visit_term(pair),
//         Rule::factor => visit_factor(pair),
//         Rule::expr => {
//             let lhs = visit_assign_expr(first);
//             if let Some(op) = maybe_op {
//                 let rhs = visit_expr(inner.next().unwrap());
//                 match op.as_span().as_str() {
//                     "==" => Expr::Eq(Box::new(lhs), Box::new(rhs)),
//                     "!=" => Expr::Ne(Box::new(lhs), Box::new(rhs)),
//                     ">" => Expr::Gt(Box::new(lhs), Box::new(rhs)),
//                     ">=" => Expr::Ge(Box::new(lhs), Box::new(rhs)),
//                     "<" => Expr::Lt(Box::new(lhs), Box::new(rhs)),
//                     "<=" => Expr::Le(Box::new(lhs), Box::new(rhs)),
//                     _ => unreachable!(),
//                 }
//             } else {
//                 lhs
//             }
//         }
//         _ => unreachable!(),
//     }
// }

fn visit_redirect(pair: Pair<Rule>) -> Redirection {
    let mut inner = pair.into_inner();
    let fd = inner.next().unwrap();
    let symbol = inner.next().unwrap();
    let target = inner.next().unwrap();

    let (direction, default_fd) = match symbol.as_span().as_str() {
        "<" => (RedirectionDirection::Input, 0),
        ">" => (RedirectionDirection::Output, 1),
        ">>" => (RedirectionDirection::Append, 1),
        _ => unreachable!(),
    };

    let fd = fd.as_span().as_str().parse().unwrap_or(default_fd);
    let target = match target.as_rule() {
        Rule::word => RedirectionType::File(visit_word(target)),
        // Rule::redirect_to_fd => {
        //     let target_fd = target
        //         .into_inner()
        //         .next()
        //         .unwrap()
        //         .as_span()
        //         .as_str()
        //         .parse()
        //         .unwrap();
        //     RedirectionType::Fd(target_fd)
        // }
        _ => unreachable!(),
    };

    Redirection {
        fd,
        direction,
        target,
    }
}

fn visit_word(pair: Pair<Rule>) -> Word {
    visit_escaped_word(pair, false)
}

fn visit_escape_sequences(pair: Pair<Rule>, escaped_chars: Option<&str>) -> String {
    let mut s = String::new();
    let mut escaped = false;
    for ch in pair.as_str().chars() {
        if escaped {
            escaped = false;
            if let Some(escaped_chars) = escaped_chars {
                if !escaped_chars.contains(ch) {
                    s.push('\\');
                }
            }
            s.push(ch);
        } else if ch == '\\' {
            escaped = true;
        } else {
            s.push(ch);
        }
    }

    s
}

fn visit_escaped_word(pair: Pair<Rule>, literal_chars: bool) -> Word {
    assert_eq!(pair.as_rule(), Rule::word);

    let mut spans = Vec::new();
    for span in pair.into_inner() {
        match span.as_rule() {
            Rule::literal_span if literal_chars => {
                let mut chars = Vec::new();
                for ch in span.into_inner() {
                    match ch.as_rule() {
                        Rule::escaped_char => {
                            let lit_ch = ch.as_str().chars().nth(1).unwrap();
                            chars.push(LiteralChar::Escaped(lit_ch))
                        }
                        Rule::unescaped_char => {
                            let lit_ch = ch.as_str().chars().next().unwrap();
                            chars.push(LiteralChar::Normal(lit_ch))
                        }
                        _ => unreachable!(),
                    }
                }
                spans.push(Span::LiteralChars(chars));
            }
            Rule::literal_span if !literal_chars => {
                spans.push(Span::Literal(visit_escape_sequences(span, None)));
            }
            _ => {
                debug!(?span);
                unimplemented!("span {:?}", span.as_rule());
            }
        }
    }

    Word(spans)
}

fn visit_command(pair: Pair<Rule>) -> Command {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::simple_command => visit_simple_command(inner),
        // Rule::if_command => visit_if_command(inner),
        // Rule::while_command => visit_while_command(inner),
        // Rule::arith_for_command => visit_arith_for_command(inner),
        // Rule::for_command => visit_for_command(inner),
        // Rule::case_command => visit_case_command(inner),
        // Rule::group => visit_group_command(inner),
        // Rule::subshell_group => visit_subshell_group_command(inner),
        // Rule::break_command => Command::Break,
        // Rule::continue_command => Command::Continue,
        // Rule::return_command => visit_return_command(inner),
        // Rule::assignment_command => visit_assignment_command(inner),
        // Rule::local_definition => visit_local_definition(inner),
        // Rule::function_definition => visit_function_definition(inner),
        // Rule::cond_ex => visit_cond_ex(inner),
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod test {
    use super::{parse, Ast, Command, Pipeline, RunIf, Span, Term, Word};

    macro_rules! literal_word_vec {
        ($($x:expr), *) => {
            vec![$( Word(vec![Span::Literal($x.to_string())]), )*]
        };
    }

    #[test]
    pub fn test_simple_commands() {
        assert_eq!(
            parse("ls -G /tmp\n"),
            Ok(Ast {
                terms: vec![Term {
                    code: "ls -G /tmp".into(),
                    pipelines: vec![Pipeline {
                        run_if: RunIf::Always,
                        commands: vec![Command::SimpleCommand {
                            argv: literal_word_vec!["ls", "-G", "/tmp"],
                            redirects: vec![],
                            assignments: vec![],
                        }],
                    }],
                }],
            })
        );
    }
}
