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
                                "smash: warning: failed to parse ~/.smash_history: at line {}",
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

    /// Appends a history to the history file.
    pub fn append(&mut self, cmd: &str) {
        if cmd.is_empty() {
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
    similary_named_offset: Option<usize>,
    input: String,
}

impl HistorySelector {
    pub fn new() -> HistorySelector {
        HistorySelector {
            offset: 0,
            similary_named_offset: None,
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

    pub fn similary_named_history(&self, history: &History) -> Option<String> {
        debug!(?self.similary_named_offset);
        self.similary_named_offset.and_then(|offset| {
            history
                .history
                .get(history.len() - (offset - 1) - 1)
                .map(|s| s.to_owned())
        })
    }

    pub fn set_similary_named_history<'a>(&mut self, history: &'a History, input: &'a str) {
        self.similary_named_offset = history
            .history
            .iter()
            .position(|h| h != input && h.starts_with(input))
            .map(|offset| history.len() - offset);
        debug!(?self.similary_named_offset, ?input);
    }

    pub fn clear_similary_named_history(&mut self) {
        self.similary_named_offset = None;
    }

    /// Selects the previous history entry. Save the current user (not yet executed)
    /// input if needed.
    pub fn prev(&mut self, history: &History, input: &str) {
        debug!(?self.offset);
        if self.offset == 0 {
            // Entering the history selection. Save the current state.state.
            self.input = input.to_string();
        }

        let hist_len = history.len();
        if let Some(offset) = history
            .history
            .iter()
            .position(|h| !input.is_empty() && h != input && h.starts_with(input))
        {
            debug!(?offset, ?input);
            self.offset = hist_len - offset;
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
