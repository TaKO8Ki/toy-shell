use crate::context_parser::{self, InputContext};
use crate::highlight::highlight;
use crossterm::cursor::{self, MoveTo};
use crossterm::event::{Event as TermEvent, KeyCode, KeyEvent, KeyModifiers};
use crossterm::style::{Attribute, Color, Print, SetAttribute, SetForegroundColor};
use crossterm::terminal::{
    self, disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use crossterm::{execute, queue};
use nix::sys::signal::{sigaction, SaFlags, SigAction, SigHandler, SigSet, Signal};
use signal_hook::{self, iterator::Signals};
use std::cmp::{max, min};
use std::io::Write;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;
use tracing::debug;

use crate::history::HistorySelector;
use crate::process::ExitStatus;
use crate::shell::Shell;

pub enum Event {
    Input(TermEvent),
    ScreenResized,
    Completion(Vec<String>),
    NoCompletion,
}

#[derive(Clone, Debug)]
struct UserInput {
    cursor: usize,
    input: String,
    indices: Vec<usize>,
    word_split: &'static str,
}

fn truncate(s: &str, len: usize) -> String {
    // TODO: Return &str
    s.chars().take(len).collect()
}

impl UserInput {
    pub fn new() -> Self {
        Self {
            cursor: 0,
            input: String::with_capacity(256),
            indices: Vec::with_capacity(256),
            word_split: " \t/",
        }
    }

    pub fn is_empty(&self) -> bool {
        self.input.is_empty()
    }

    pub fn len(&self) -> usize {
        self.indices.len()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn as_str(&self) -> &str {
        self.input.as_str()
    }

    pub fn clear(&mut self) {
        self.cursor = 0;
        self.input.clear();
        self.indices.clear();
    }

    pub fn reset(&mut self, input: String) {
        self.input = input;
        self.update_indices();
        self.move_to_end();
    }

    pub fn insert(&mut self, ch: char) {
        self.input.insert(self.byte_index(), ch);
        self.update_indices();
        self.cursor += 1;
    }

    pub fn delete(&mut self) {
        if self.cursor < self.len() {
            self.input.remove(self.byte_index());
            self.update_indices();
        }
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.input.remove(self.byte_index());
            self.update_indices();
        }
    }

    pub fn insert_str(&mut self, string: &str) {
        self.input.insert_str(self.byte_index(), string);
        self.update_indices();
        self.cursor += string.chars().count();
    }

    fn byte_index(&self) -> usize {
        if self.cursor == self.indices.len() {
            self.input.len()
        } else {
            self.indices[self.cursor]
        }
    }

    fn update_indices(&mut self) {
        self.indices.clear();
        for index in self.input.char_indices() {
            self.indices.push(index.0);
        }
    }

    pub fn replace_range(&mut self, range: Range<usize>, replace_with: &str) {
        debug!(?range, ?self.input, ?replace_with);
        let cursor = range.start + replace_with.chars().count();
        self.input.replace_range(range, replace_with);
        debug!(?self.input);
        self.update_indices();
        self.cursor = cursor;
    }

    pub fn move_by(&mut self, offset: isize) {
        if offset < 0 {
            self.cursor = self.cursor.saturating_sub(offset.abs() as usize);
        } else {
            self.cursor = min(self.len(), self.cursor + offset.abs() as usize);
        }
    }

    pub fn move_to_begin(&mut self) {
        self.cursor = 0;
    }

    pub fn move_to_end(&mut self) {
        self.cursor = self.len();
    }
}

pub struct SmashState {
    columns: usize,
    shell: Shell,
    input: UserInput,
    input_stack: Vec<String>,
    prompt_len: usize,
    clear_above: usize,
    clear_below: usize,
    exited: Option<ExitStatus>,
    do_complete: bool,
    input_ctx: InputContext,
    completions: Vec<String>,
    filtered_completions: Vec<String>,
    selected_completion: usize,
    completions_show_from: usize,
    completions_height: usize,
    completions_per_line: usize,
    lines: usize,
    // history
    history_selector: HistorySelector,
}

impl Drop for SmashState {
    fn drop(&mut self) {
        disable_raw_mode().ok();
    }
}

impl SmashState {
    pub fn new(shell: Shell) -> Self {
        Self {
            shell,
            input: UserInput::new(),
            clear_above: 0,
            clear_below: 0,
            prompt_len: 0,
            columns: 0,
            input_stack: Vec::new(),
            exited: None,
            do_complete: false,
            input_ctx: context_parser::parse("", 0),
            completions: Vec::new(),
            filtered_completions: Vec::new(),
            selected_completion: 0,
            completions_show_from: 0,
            completions_height: 0,
            completions_per_line: 0,
            lines: 0,
            history_selector: HistorySelector::new(),
        }
    }

    pub fn run(&mut self) {
        let (tx, rx) = mpsc::channel();
        let tx2 = tx.clone();
        std::thread::spawn(move || {
            let signals = Signals::new(&[signal_hook::SIGWINCH]).unwrap();
            for signal in signals {
                match signal {
                    signal_hook::SIGWINCH => {
                        tx2.send(Event::ScreenResized).ok();
                    }
                    _ => {
                        tracing::warn!("unhandled signal: {}", signal);
                    }
                }
            }

            unreachable!();
        });

        enable_raw_mode().ok();
        self.render_prompt();

        let action = SigAction::new(SigHandler::SigIgn, SaFlags::empty(), SigSet::empty());
        unsafe {
            sigaction(Signal::SIGINT, &action).expect("failed to sigaction");
            sigaction(Signal::SIGQUIT, &action).expect("failed to sigaction");
            sigaction(Signal::SIGTSTP, &action).expect("failed to sigaction");
            sigaction(Signal::SIGTTIN, &action).expect("failed to sigaction");
            sigaction(Signal::SIGTTOU, &action).expect("failed to sigaction");
        }

        loop {
            let mut started_at = None;

            match crossterm::event::poll(Duration::from_millis(100)) {
                Ok(true) => loop {
                    if let Ok(ev) = crossterm::event::read() {
                        self.handle_event(Event::Input(ev))
                    }

                    match crossterm::event::poll(Duration::from_millis(0)) {
                        Ok(true) => (), // Continue reading stdin.
                        _ => break,
                    }
                },
                _ => {
                    if let Ok(ev) = rx.try_recv() {
                        started_at = Some(std::time::SystemTime::now());
                        self.handle_event(ev);
                    }
                }
            }

            if self.do_complete {
                let is_argv0 = if let Some(current_span) = self.input_ctx.current_span {
                    matches!(
                        &self.input_ctx.spans[current_span],
                        context_parser::Span::Argv0(_)
                    )
                } else {
                    false
                };

                debug!(?is_argv0);
                if is_argv0 {
                    // Command name completion.
                    let argv0 = self.current_span_text().unwrap();
                    debug!(?argv0);
                    let comps = if argv0.starts_with('/')
                        || argv0.starts_with('.')
                        || argv0.starts_with('~')
                    {
                        path_completion(argv0, false)
                    } else {
                        self.shell.path_table().to_vec()
                    };
                    tx.send(Event::Completion(comps)).ok();
                } else {
                    let pattern = self.current_span_text().unwrap_or("");
                    let entries = path_completion(pattern, self.input_ctx.words[0] == "cd");
                    tx.send(Event::Completion(entries)).ok();
                }

                self.do_complete = false;
            }
        }
    }

    fn current_span_text(&self) -> Option<&str> {
        if let Some(current_span_index) = self.input_ctx.current_span {
            match &self.input_ctx.spans[current_span_index] {
                context_parser::Span::Literal(literal) | context_parser::Span::Argv0(literal) => {
                    return Some(literal);
                }
                _ => {}
            };
        }

        None
    }

    fn select_completion(&mut self) {
        if let Some(current_span) = &self.input_ctx.current_literal {
            if let Some(selected) = self.filtered_completions.get(self.selected_completion) {
                self.input.replace_range(current_span.clone(), &selected);
            }

            self.clear_completions();
        }
    }

    fn hide_completions(&mut self) {
        let mut stdout = std::io::stdout();
        if self.completions_height > 0 {
            queue!(stdout, cursor::Hide).ok();

            let comps_y_diff = self.clear_below - self.completions_height;
            if comps_y_diff > 0 {
                queue!(stdout, cursor::MoveDown(comps_y_diff as u16)).ok();
            }

            for _ in 0..self.completions_height {
                queue!(stdout, cursor::MoveDown(1), Clear(ClearType::CurrentLine)).ok();
            }

            queue!(
                stdout,
                cursor::MoveUp((comps_y_diff + self.completions_height) as u16),
                cursor::Show,
            )
            .ok();

            stdout.flush().ok();
        }
    }

    fn completion_mode(&self) -> bool {
        !self.completions.is_empty()
    }

    fn clear_completions(&mut self) {
        self.completions.clear();
    }

    fn update_completion_entries(&mut self, entries: Vec<String>) {
        self.completions = entries;
        self.completions_show_from = 0;
        self.filter_completion_entries();

        if self.filtered_completions.len() == 1 {
            self.select_completion();
            self.reparse_input_ctx();
        }

        self.print_user_input();
    }

    fn filter_completion_entries(&mut self) {
        self.filtered_completions = self
            .completions
            .iter()
            .filter(|comp| {
                self.current_span_text().map_or(false, |text| {
                    !self.input.is_empty() && comp.starts_with(text)
                })
            })
            .map(|s| s.to_string().replace(" ", "\\ "))
            .collect();
        debug!(?self.filtered_completions);
        self.selected_completion = min(
            self.selected_completion,
            self.filtered_completions.len().saturating_sub(1),
        );
    }

    fn reparse_input_ctx(&mut self) {
        self.input_ctx = context_parser::parse(self.input.as_str(), self.input.cursor());
    }

    pub fn handle_event(&mut self, ev: Event) {
        match ev {
            Event::Input(input) => {
                if let TermEvent::Key(key) = input {
                    self.handle_key_event(&key)
                }
            }
            Event::ScreenResized => {
                debug!("screen resize");
                let screen_size = terminal::size().unwrap();
                self.columns = screen_size.0 as usize;
                self.lines = screen_size.1 as usize;
            }
            Event::NoCompletion => {
                todo!()
            }
            Event::Completion(comps) => {
                if comps.is_empty() {
                    debug!("empty completions")
                } else {
                    debug!(?comps);
                    self.update_completion_entries(comps);
                }
            }
        }
    }

    pub fn handle_key_event(&mut self, ev: &KeyEvent) {
        let mut needs_redraw = true;
        match (ev.code, ev.modifiers) {
            // completion
            (KeyCode::Esc, KeyModifiers::NONE)
            | (KeyCode::Char('q'), KeyModifiers::NONE)
            | (KeyCode::Char('c'), KeyModifiers::CONTROL)
            | (KeyCode::Backspace, KeyModifiers::NONE)
                if self.completion_mode() =>
            {
                self.clear_completions();
            }
            (KeyCode::Enter, KeyModifiers::NONE) if self.completion_mode() => {
                self.select_completion()
            }
            (KeyCode::Left, KeyModifiers::NONE) | (KeyCode::BackTab, KeyModifiers::SHIFT)
                if self.completion_mode() =>
            {
                self.selected_completion = self.selected_completion.saturating_sub(1);
            }
            (KeyCode::Up, KeyModifiers::NONE) | (KeyCode::Char('p'), KeyModifiers::CONTROL)
                if self.completion_mode() =>
            {
                self.selected_completion = self
                    .selected_completion
                    .saturating_sub(self.completions_per_line);
            }
            (KeyCode::Down, KeyModifiers::NONE) | (KeyCode::Char('n'), KeyModifiers::CONTROL)
                if self.completion_mode() =>
            {
                self.selected_completion = min(
                    self.selected_completion + self.completions_per_line,
                    self.filtered_completions.len().saturating_sub(1),
                );
            }
            (KeyCode::Right, KeyModifiers::NONE) | (KeyCode::Tab, KeyModifiers::NONE)
                if self.completion_mode() =>
            {
                if self.filtered_completions.is_empty() {
                    self.clear_completions();
                } else {
                    self.selected_completion = min(
                        self.selected_completion + 1,
                        self.filtered_completions.len().saturating_sub(1),
                    );
                }
            }
            (KeyCode::Tab, KeyModifiers::NONE) => {
                self.do_complete = true;
            }
            // history
            (KeyCode::Up, KeyModifiers::NONE) => {
                self.history_selector
                    .prev(self.shell.history(), self.input.as_str());
                if let Some(line) = self.history_selector.current(self.shell.history()) {
                    self.input.reset(line);
                }
            }
            (KeyCode::Down, KeyModifiers::NONE) => {
                self.history_selector.next();
                debug!(?self.input, "down");
                if let Some(line) = self.history_selector.current(self.shell.history()) {
                    self.input.reset(line);
                }
            }
            // misc
            (KeyCode::Backspace, KeyModifiers::NONE) => {
                self.input.backspace();
                self.history_selector.clear_similary_named_history();
            }
            (KeyCode::Enter, KeyModifiers::NONE) => {
                debug!("enter");
                let mut stdout = std::io::stdout();
                execute!(stdout, Clear(ClearType::UntilNewLine)).ok();
                self.run_command();
                needs_redraw = false;
            }
            (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                self.clear_completions();
                self.input.move_to_begin();
            }
            (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
                self.clear_completions();
                self.input.move_to_end();
            }
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                let mut stdout = std::io::stdout();
                execute!(stdout, Clear(ClearType::UntilNewLine)).ok();
                execute!(stdout, Print("\r\n")).ok();
                self.render_prompt();
                self.input.clear();
                self.history_selector.clear_similary_named_history();
            }
            (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                if self.input.is_empty() {
                    self.exited = Some(ExitStatus::ExitedWith(0));
                } else {
                    self.input.delete();
                }
            }
            (KeyCode::Left, KeyModifiers::NONE) => {
                self.input.move_by(-1);
            }
            (KeyCode::Right, KeyModifiers::NONE) => {
                match self
                    .history_selector
                    .similary_named_history(self.shell.history())
                {
                    Some(history) => {
                        self.input.reset(history);
                        self.history_selector.reset();
                    }
                    None => {
                        self.input.move_by(1);
                    }
                }
            }
            (KeyCode::Char(ch), KeyModifiers::NONE) => {
                debug!(
                    "history={:?}",
                    self.history_selector
                        .similary_named_history(self.shell.history())
                );
                self.input.insert(ch);
                self.history_selector
                    .set_similary_named_history(self.shell.history(), self.input.as_str());
            }
            (KeyCode::Char(ch), KeyModifiers::SHIFT) => {
                self.input.insert(ch);
            }
            _ => (),
        }

        if needs_redraw {
            self.reparse_input_ctx();
            self.filter_completion_entries();
            self.print_user_input();
        }
    }

    pub fn render_prompt(&mut self) {
        let screen_size = terminal::size().unwrap();
        self.columns = screen_size.0 as usize;
        self.lines = screen_size.1 as usize;

        tracing::debug!(?self.columns);

        let mut stdout = std::io::stdout();
        queue!(
            stdout,
            SetAttribute(Attribute::Bold),
            SetAttribute(Attribute::Reverse),
            Print("$"),
            SetAttribute(Attribute::Reset),
            Print(&format!(
                "{space:>width$}\r",
                space = " ",
                width = self.columns - 1
            ))
        )
        .ok();

        let (mut prompt_str, mut prompt_len) = (String::new(), 0);
        if let Ok(current_dir) = std::env::current_dir() {
            let mut path = current_dir.to_str().unwrap().to_string();

            // "/Users/chandler/games/doom" -> "~/venus/games/doom"
            if let Some(home_dir) = dirs::home_dir() {
                let home_dir = home_dir.to_str().unwrap();
                if path.starts_with(&home_dir) {
                    path = path.replace(home_dir, "~");
                }
            }

            prompt_str.push_str(&path);
        }
        prompt_str.push_str(" $ ");
        queue!(stdout, Print(prompt_str.replace("\n", "\r\n"))).ok();
        prompt_len += prompt_str.len();
        stdout.flush().unwrap();

        self.prompt_len = prompt_len;
    }

    fn push_buffer_stack(&mut self) {
        self.input_stack.push(self.input.as_str().to_owned());
        self.input.clear();
    }

    fn run_command(&mut self) {
        self.history_selector.clear_similary_named_history();
        self.history_selector.reset();

        self.print_user_input();
        self.hide_completions();

        execute!(std::io::stdout(), Print("\r\n")).ok();
        disable_raw_mode().ok();
        self.shell.run_str(self.input.as_str());
        enable_raw_mode().ok();

        self.shell.history_mut().append(self.input.as_str());
        self.input.clear();
        self.clear_above = 0;
        self.clear_below = 0;

        if let Some(input) = self.input_stack.pop() {
            self.input.insert_str(&input);
        }

        self.reparse_input_ctx();
        self.render_prompt();
        self.print_user_input();

        debug!(
            "history={:?}",
            self.history_selector
                .similary_named_history(&self.shell.history())
        );
    }

    fn print_user_input(&mut self) {
        if cfg!(test) {
            // Do nothing in tests.
            return;
        }

        let mut stdout = std::io::stdout();

        // Hide the cursor to prevent annoying flickering.
        queue!(stdout, cursor::Hide).ok();

        // Clear the previous user input and completions.
        // TODO: Don't clear the texts; overwrite instead to prevent flickering.
        if self.clear_below > 0 {
            for _ in 0..self.clear_below {
                queue!(stdout, cursor::MoveDown(1), Clear(ClearType::CurrentLine)).ok();
            }

            queue!(stdout, cursor::MoveUp(self.clear_below as u16)).ok();
        }

        for _ in 0..self.clear_above {
            queue!(stdout, Clear(ClearType::CurrentLine), cursor::MoveUp(1)).ok();
        }

        if self.clear_above > 0 {
            // Redraw the prompt since it has been cleared.
            let (prompt_str, _) = (String::new(), 0);
            queue!(stdout, Print("\r"), Print(prompt_str.replace("\n", "\r\n"))).ok();
        }

        // Print the highlighted input.
        let h = highlight(&self.input_ctx, &mut self.shell);
        queue!(
            stdout,
            Print("\r"),
            cursor::MoveRight(self.prompt_len as u16),
            Clear(ClearType::UntilNewLine),
            Print(h.replace("\n", "\r\n"))
        )
        .ok();

        // Print the first history item;
        if let Some(history) = self.similary_named_history() {
            debug!(?history, ?self.input_ctx.input);
            if let Some(suffix) = history.strip_prefix(&self.input_ctx.input) {
                queue!(
                    stdout,
                    SetForegroundColor(Color::DarkGrey),
                    Print(suffix),
                    SetAttribute(Attribute::Reset),
                )
                .ok();
            }
        }

        // Handle the case when the cursor is at the end of a line.
        let current_x = self.prompt_len + self.input.len();
        if current_x % self.columns == 0 {
            queue!(stdout, Print("\r\n")).ok();
        }

        // Print a notification message.
        // if let Some(notification) = &self.notification {
        //     queue!(
        //         stdout,
        //         Print("\r\n"),
        //         SetForegroundColor(Color::Yellow),
        //         SetAttribute(Attribute::Bold),
        //         Print("[!] "),
        //         Print(notification),
        //         SetAttribute(Attribute::Reset),
        //         Clear(ClearType::UntilNewLine),
        //     )
        //     .ok();
        // }

        // let notification_height = if self.notification.is_some() { 1 } else { 0 };
        // let input_height = current_x / self.columns + notification_height;
        let input_height = current_x / self.columns;

        let mut completions_height = 0;
        if self.completion_mode() {
            // Determine the number of columns and its width of completions.
            let mut longest = 0;
            for comp in self.completions.iter() {
                longest = max(longest, comp.len() + 1);
            }

            let num_columns = max(1, self.columns / longest);
            let column_width = self.columns / num_columns;

            // Move `self.completions_show_from`.
            let completions_height_max = self.lines - input_height - 1;
            let num_comps_max = (completions_height_max - 1) * num_columns;
            if self.selected_completion < self.completions_show_from {
                self.completions_show_from = (self.selected_completion / num_columns) * num_columns;
            }

            if self.selected_completion >= self.completions_show_from + num_comps_max {
                self.completions_show_from =
                    (self.selected_completion / num_columns + 1) * num_columns - num_comps_max;
            }

            // Print completions.
            let mut remaining = self.filtered_completions.len() - self.completions_show_from;
            let iter = self
                .filtered_completions
                .iter()
                .skip(self.completions_show_from);
            for (i, comp) in iter.enumerate() {
                if i % num_columns == 0 {
                    if completions_height == completions_height_max - 1 {
                        break;
                    }

                    queue!(stdout, Print("\r\n")).ok();
                    completions_height += 1;
                }

                let margin = column_width - min(comp.len(), column_width);
                if self.completions_show_from + i == self.selected_completion {
                    queue!(
                        stdout,
                        SetAttribute(Attribute::Reverse),
                        Print(truncate(comp, self.columns)),
                        SetAttribute(Attribute::NoReverse),
                        cursor::MoveRight(margin as u16),
                    )
                    .ok();
                } else {
                    // if let Some(ThemeColor::DirColor) = color {
                    //     self.dircolor.write(&mut stdout, Path::new(comp)).ok();
                    // }

                    queue!(
                        stdout,
                        Print(truncate(comp, self.columns)),
                        SetAttribute(Attribute::Reset),
                        cursor::MoveRight(margin as u16)
                    )
                    .ok();
                }

                remaining -= 1;
            }

            if remaining > 0 {
                completions_height += 2;
                queue!(
                    stdout,
                    Clear(ClearType::UntilNewLine),
                    Print("\r\n"),
                    SetAttribute(Attribute::Reverse),
                    Print(" "),
                    Print(remaining),
                    Print(" more "),
                    SetAttribute(Attribute::Reset),
                )
                .ok();
            }

            self.completions_per_line = num_columns;
        }

        // Move the cursor to the correct position.
        let cursor_y = (self.prompt_len + self.input.cursor()) / self.columns;
        let cursor_x = (self.prompt_len + self.input.cursor()) % self.columns;
        let cursor_y_diff = (input_height - cursor_y) + completions_height;
        if cursor_y_diff > 0 {
            queue!(stdout, cursor::MoveUp(cursor_y_diff as u16),).ok();
        }

        queue!(stdout, Print("\r")).ok();
        if cursor_x > 0 {
            queue!(stdout, cursor::MoveRight(cursor_x as u16),).ok();
        }

        queue!(stdout, cursor::Show).ok();
        self.clear_above = cursor_y;
        self.clear_below = input_height - cursor_y + completions_height;
        self.completions_height = completions_height;
        stdout.flush().ok();
    }

    pub fn similary_named_history(&self) -> Option<String> {
        self.history_selector
            .similary_named_history(self.shell.history())
    }
}

fn path_completion(pattern: &str, only_dirs: bool) -> Vec<String> {
    let home_dir = dirs::home_dir().unwrap();
    let current_dir = std::env::current_dir().unwrap();
    let mut dir = if pattern.is_empty() {
        current_dir.clone()
    } else if let Some(pattern) = pattern.strip_prefix('~') {
        home_dir.join(&pattern.trim_start_matches('/'))
    } else {
        PathBuf::from(pattern)
    };

    // "/usr/loca" -> "/usr"
    dir = if dir.is_dir() {
        dir
    } else {
        dir.pop();
        if dir.to_str().unwrap().is_empty() {
            current_dir.clone()
        } else {
            dir
        }
    };

    debug!(
        "path_completion: dir={}, pattern='{}', only_dirs={}",
        dir.display(),
        pattern,
        only_dirs
    );
    match std::fs::read_dir(&dir) {
        Ok(files) => {
            let mut entries = Vec::new();
            for file in files {
                let file = file.unwrap();
                if only_dirs && !file.file_type().unwrap().is_dir() {
                    continue;
                }

                let path = file.path();

                // Ignore dotfiles unless the pattern contains ".".
                if !pattern.starts_with('.') && !pattern.contains("/.") {
                    if let Some(filename) = path.file_name() {
                        if let Some(filename) = filename.to_str() {
                            if filename.starts_with('.') {
                                continue;
                            }
                        }
                    }
                }

                let (prefix, relpath) = if pattern.starts_with('~') {
                    ("~/", path.strip_prefix(&home_dir).unwrap())
                } else if pattern.starts_with('/') {
                    ("/", path.strip_prefix("/").unwrap())
                } else {
                    ("", path.strip_prefix(&current_dir).unwrap_or(&path))
                };

                let comp = format!("{}{}", prefix, relpath.to_str().unwrap());
                entries.push(comp);
            }

            entries.sort();
            entries
        }
        Err(err) => {
            debug!("failed to readdir '{}': {}", dir.display(), err);
            vec![]
        }
    }
}
