use std::collections::HashMap;
use std::fs::read_dir;

pub struct PathTable {
    /// `$PATH`
    path: String,
    /// Key is command name and value is absolute path to the executable.
    table: HashMap<String, String>,
}

impl PathTable {
    pub fn new() -> PathTable {
        PathTable {
            path: String::new(),
            table: HashMap::new(),
        }
    }

    pub fn scan(&mut self, path: &str) {
        self.path = path.to_string();
        self.rehash();
    }

    pub fn to_vec(&self) -> Vec<String> {
        self.table.clone().into_keys().collect()
    }

    pub fn rehash(&mut self) {
        self.table.clear();
        for bin_dir in self.path.split(':').rev() {
            if let Ok(files) = read_dir(bin_dir) {
                for entry in files {
                    let file = entry.unwrap();
                    let basename = file.file_name().to_str().unwrap().to_owned();
                    let fullpath = file.path().to_str().unwrap().to_owned();
                    self.table.insert(basename.clone(), fullpath);
                }
            }
        }
    }

    pub fn lookup(&self, cmd: &str) -> Option<&str> {
        self.table.get(cmd).map(String::as_str)
    }
}
