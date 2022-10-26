use std::collections::HashMap;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub enum Value {
    String(String),
    Array(Vec<String>),
    // TODO: support function
}

#[derive(Debug)]
pub struct Variable {
    // The inner value. `None` represents *null*.
    value: Option<Value>,
}

impl Variable {
    pub fn new(value: Option<Value>) -> Variable {
        Variable { value }
    }

    #[inline]
    pub fn value(&self) -> &Option<Value> {
        &self.value
    }

    pub fn as_str(&self) -> &str {
        match &self.value {
            Some(Value::String(value)) => value,
            Some(Value::Array(elems)) => match elems.get(0) {
                Some(elem) => elem.as_str(),
                _ => "",
            },
            None => "",
        }
    }
}

pub struct Frame {
    /// key: variable name, value: varible map.
    vars: HashMap<String, Rc<Variable>>,
}

impl Frame {
    pub fn new() -> Frame {
        Frame {
            vars: HashMap::new(),
        }
    }

    pub fn get(&self, key: &str) -> Option<Rc<Variable>> {
        self.vars.get(key).cloned()
    }

    pub fn set(&mut self, key: &str, value: Value) {
        self.vars
            .insert(key.into(), Rc::new(Variable::new(Some(value))));
    }
}
