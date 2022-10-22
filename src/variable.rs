use crate::parser;

pub enum Value {
    String(String),
    Array(Vec<String>),
    Function(parser::Command),
}
