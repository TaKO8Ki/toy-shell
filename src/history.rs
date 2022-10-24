use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use tracing::debug;

/// Command history.
#[derive(Debug)]
pub struct History {
    path: PathBuf,
    history: Vec<String>,
    path2cwd: HashMap<String, PathBuf>,
}

impl History {
    pub fn new(history_file: &Path) -> History {
        // Loads the history file.
        let mut warned = false;
        let mut path2cwd = HashMap::new();
        let mut history = Vec::new();
        if let Ok(file) = File::open(history_file) {
            for (i, line) in BufReader::new(file).lines().enumerate() {
                if let Ok(line) = line {
                    let cwd = line.split('\t').nth(1);
                    let cmd = line.split('\t').nth(2);
                    match (cwd, cmd, warned) {
                        (Some(cwd), Some(cmd), _) => {
                            path2cwd.insert(cmd.to_string(), PathBuf::from(cwd));
                            history.push(cmd.to_string());
                        }
                        (_, _, false) => {
                            smash_err!(
                                "nsh: warning: failed to parse ~/.nsh_history: at line {}",
                                i + 1
                            );
                            warned = true;
                        }
                        (_, _, _) => (),
                    }
                }
            }
        }

        History {
            path: history_file.to_owned(),
            history,
            path2cwd,
        }
    }

    pub fn len(&self) -> usize {
        self.history.len()
    }

    // pub fn nth_last(&self, nth: usize) -> Option<String> {
    //     self.history.nth_last(nth)
    // }

    pub fn search(&self, query: &str, filter_by_cwd: bool) -> Vec<String> {
        if filter_by_cwd {
            let cwd = std::env::current_dir().unwrap();
            self.history
                .iter()
                .filter(|cmd| match self.path2cwd.get(*cmd) {
                    Some(path) if *path == cwd => cmd.starts_with(query),
                    Some(path) => {
                        debug!("path='{}' {}", path.display(), cwd.display());
                        false
                    }
                    _ => false,
                })
                .cloned()
                .collect()
        } else {
            self.history
                .iter()
                .filter(|cmd| cmd.starts_with(query))
                .cloned()
                .collect()
        }
    }

    /// Appends a history to the history file.
    pub fn append(&mut self, cmd: &str) {
        if cmd.is_empty() {
            return;
        }

        if cmd.len() < 8 {
            return;
        }

        // Ignore if `cmd` is same as the last command.
        if let Some(last) = self.history.last() {
            if last.as_str() == cmd {
                return;
            }
        }

        let cwd = std::env::current_dir().unwrap();
        if let Ok(mut file) = OpenOptions::new().append(true).open(&self.path) {
            let time = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("failed to get the UNIX timestamp")
                .as_secs() as usize;
            let dir = cwd.to_str().unwrap().to_owned();
            file.write(format!("{}\t{}\t{}\n", time, dir, cmd).as_bytes())
                .ok();
        }

        self.history.push(cmd.to_string());
        self.path2cwd.insert(cmd.to_string(), cwd);
    }
}

pub struct HistorySelector {
    offset: usize,
    input: String,
}

impl HistorySelector {
    pub fn new() -> HistorySelector {
        HistorySelector {
            offset: 0,
            input: String::new(),
        }
    }

    pub fn reset(&mut self) {
        self.offset = 0;
    }

    pub fn current(&self, history: &History) -> Option<String> {
        debug!(?history, ?self.offset);
        if self.offset == 0 {
            Some(self.input.clone())
        } else {
            history
                .history
                .get(history.len() - (self.offset - 1) - 1)
                .map(|s| s.to_owned())
        }
    }

    /// Selects the previous history entry. Save the current user (not yet executed)
    /// input if needed.
    pub fn prev(&mut self, history: &History, input: &str) {
        // Entering the history selection. Save the current state.state.
        self.input = input.to_string();
        let hist_len = history.len();
        if !self.input.is_empty() {
            if let Some(offset) = history.history.iter().position(|h| h.starts_with(input)) {
                debug!(?offset, ?input);
                self.offset = hist_len - offset;
            }
        } else {
            self.offset += 1;
        }

        if self.offset >= hist_len {
            self.offset = hist_len;
        }
        debug!(?self.offset);
    }

    /// Select the next history entry.
    pub fn next(&mut self) {
        if self.offset > 0 {
            self.offset -= 1;
        }
    }
}
