use super::{BuiltinCommand, BuiltinCommandContext};
use crate::parser;
use crate::process::ExitStatus;
use pest::Parser;
use pest_derive::Parser;
use tracing::debug;

#[derive(Parser)]
#[grammar = "builtins/alias.pest"]
struct AliasParser;

fn parse_alias(alias: &str) -> Result<(String, String), parser::ParseError> {
    AliasParser::parse(Rule::alias, alias)
        .map_err(|err| parser::ParseError::Fatal(err.to_string()))
        .map(|mut pairs| {
            let mut inner = pairs.next().unwrap().into_inner();
            let name = inner.next().unwrap().as_span().as_str().to_owned();
            let body = inner.next().unwrap().as_str().to_owned();
            (name, body)
        })
}

pub struct Alias;

impl BuiltinCommand for Alias {
    fn run(&self, ctx: BuiltinCommandContext) -> ExitStatus {
        debug!("alias: argv={:?}", ctx.argv);
        if let Some(alias) = ctx.argv.get(1) {
            match parse_alias(alias) {
                Ok((name, body)) => {
                    debug!(?name, ?body, "add a alias");
                    ctx.shell.add_alias(&name, body);
                    return ExitStatus::ExitedWith(0);
                }
                Err(parser::ParseError::Fatal(err)) => {
                    smash_err!("alias: {}", err);
                    return ExitStatus::ExitedWith(1);
                }
                Err(parser::ParseError::Empty) => {
                    smash_err!("alias: alias can't be empty string");
                    return ExitStatus::ExitedWith(1);
                }
            }
        }
        ExitStatus::ExitedWith(0)
    }
}
