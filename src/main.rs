use crossterm::cursor::{self, MoveTo};
use crossterm::event::{
    read, DisableMouseCapture, EnableMouseCapture, Event as TermEvent, KeyCode, KeyEvent,
    KeyModifiers,
};
use crossterm::style::{Attribute, Color, Print, SetAttribute, SetForegroundColor};
use crossterm::terminal::{
    self, disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use crossterm::{execute, queue};
use signal_hook::{self, iterator::Signals};
use std::io::Write;
use std::sync::mpsc;
use std::time::Duration;
use tracing_subscriber;

mod parser;
struct Shell {}

enum Event {
    Input(TermEvent),
    ScreenResized,
    NoCompletion,
}

impl Shell {
    fn new() -> Self {
        Self {}
    }
}

struct SmashState {
    columns: usize,
    shell: Shell,
    input: UserInput,
    input_stack: Vec<String>,
    prompt_len: usize,
    clear_above: usize,
    clear_below: usize,
}

#[derive(Clone)]
struct UserInput {
    cursor: usize,
    input: String,
    indices: Vec<usize>,
    word_split: &'static str,
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
}

impl SmashState {
    fn new(shell: Shell) -> Self {
        Self {
            shell,
            input: UserInput::new(),
            clear_above: 0,
            clear_below: 0,
            prompt_len: 0,
            columns: 0,
            input_stack: Vec::new()
        }
    }

    fn render_prompt(&mut self) {
        let screen_size = terminal::size().unwrap();
        self.columns = screen_size.0 as usize;

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

            prompt_len += path.len();
            prompt_str.push_str(&path);
        }
        prompt_str.push_str(" $ ");
        queue!(stdout, Print(prompt_str.replace("\n", "\r\n"))).ok();
        stdout.flush().unwrap();
    }

    fn push_buffer_stack(&mut self) {
        self.input_stack.push(self.input.as_str().to_owned());
        self.input.clear();
    }

    fn run_command(&mut self) {
        self.print_user_input();
        // self.hide_completions();

        print!("\r\n");
        disable_raw_mode().ok();
        self.shell.run_str(self.input.as_str());
        enable_raw_mode().ok();
        // check_background_jobs(&mut self.shell);

        // self.shell.history_mut().append(self.input.as_str());
        self.input.clear();
        // self.history_selector.reset();
        self.clear_above = 0;
        self.clear_below = 0;

        if let Some(input) = self.input_stack.pop() {
            self.input.insert_str(&input);
        }

        // self.reparse_input_ctx();
        self.render_prompt();
        self.print_user_input();
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

        // if self.clear_above > 0 {
        //     // Redraw the prompt since it has been cleared.
        //     let (prompt_str, _) = self.render_prompt();
        //     queue!(stdout, Print("\r"), Print(prompt_str.replace("\n", "\r\n"))).ok();
        // }

        // Print the highlighted input.
        // let h = highlight::highlight(&self.input_ctx, &mut self.shell);
        queue!(
            stdout,
            Print("\r"),
            cursor::MoveRight(self.prompt_len as u16),
            Clear(ClearType::UntilNewLine),
            Print(self.input.input.replace("\n", "\r\n"))
        )
        .ok();

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

        let mut comps_height = 0;
        // if self.completion_mode() {
        //     // Determine the number of columns and its width of completions.
        //     let mut longest = 0;
        //     for (_, comp) in self.completions.iter() {
        //         longest = max(longest, comp.len() + 1);
        //     }

        //     let num_columns = max(1, self.columns / longest);
        //     let column_width = self.columns / num_columns;

        //     // Move `self.comps_show_from`.
        //     let comps_height_max = self.lines - input_height - 1;
        //     let num_comps_max = (comps_height_max - 1) * num_columns;
        //     if self.comp_selected < self.comps_show_from {
        //         self.comps_show_from = (self.comp_selected / num_columns) * num_columns;
        //     }

        //     if self.comp_selected >= self.comps_show_from + num_comps_max {
        //         self.comps_show_from =
        //             (self.comp_selected / num_columns + 1) * num_columns - num_comps_max;
        //     }

        //     // Print completions.
        //     let mut remaining = self.comps_filtered.len() - self.comps_show_from;
        //     let iter = self.comps_filtered.iter().skip(self.comps_show_from);
        //     for (i, (color, comp)) in iter.enumerate() {
        //         if i % num_columns == 0 {
        //             if comps_height == comps_height_max - 1 {
        //                 break;
        //             }

        //             queue!(stdout, Print("\r\n")).ok();
        //             comps_height += 1;
        //         }

        //         let margin = column_width - min(comp.len(), column_width);
        //         if self.comps_show_from + i == self.comp_selected {
        //             queue!(
        //                 stdout,
        //                 SetAttribute(Attribute::Reverse),
        //                 Print(truncate(comp, self.columns)),
        //                 SetAttribute(Attribute::NoReverse),
        //                 cursor::MoveRight(margin as u16),
        //             )
        //             .ok();
        //         } else {
        //             if let Some(ThemeColor::DirColor) = color {
        //                 self.dircolor.write(&mut stdout, Path::new(comp)).ok();
        //             }

        //             queue!(
        //                 stdout,
        //                 Print(truncate(comp, self.columns)),
        //                 SetAttribute(Attribute::Reset),
        //                 cursor::MoveRight(margin as u16)
        //             )
        //             .ok();
        //         }

        //         remaining -= 1;
        //     }

        //     if remaining > 0 {
        //         comps_height += 2;
        //         queue!(
        //             stdout,
        //             Clear(ClearType::UntilNewLine),
        //             Print("\r\n"),
        //             SetAttribute(Attribute::Reverse),
        //             Print(" "),
        //             Print(remaining),
        //             Print(" more "),
        //             SetAttribute(Attribute::Reset),
        //         )
        //         .ok();
        //     }

        //     self.comps_per_line = num_columns;
        // }

        // Move the cursor to the correct position.
        let cursor_y = (self.prompt_len + self.input.cursor()) / self.columns;
        let cursor_x = (self.prompt_len + self.input.cursor()) % self.columns;
        let cursor_y_diff = (input_height - cursor_y) + comps_height;
        if cursor_y_diff > 0 {
            queue!(stdout, cursor::MoveUp(cursor_y_diff as u16),).ok();
        }

        queue!(stdout, Print("\r")).ok();
        if cursor_x > 0 {
            queue!(stdout, cursor::MoveRight(cursor_x as u16),).ok();
        }

        queue!(stdout, cursor::Show).ok();
        self.clear_above = cursor_y;
        self.clear_below = input_height - cursor_y + comps_height;
        // self.comps_height = comps_height;
        stdout.flush().ok();
    }
}

fn main() {
    tracing_subscriber::fmt::init();

    let mut shell = Shell::new();
    let mut state = SmashState::new(shell);

    for (key, value) in std::env::vars() {}

    enable_raw_mode().ok();
    state.render_prompt();

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

    'main: loop {
        let mut started_at = None;

        match crossterm::event::poll(Duration::from_millis(100)) {
            Ok(true) => loop {
                tracing::debug!("eventtttttttttttttttttt");
                if let Ok(TermEvent::Key(ev)) = crossterm::event::read() {
                    match (ev.code, ev.modifiers) {
                        (KeyCode::Char('q'), KeyModifiers::NONE) => break 'main,
                        (KeyCode::Enter, KeyModifiers::NONE) => {
                            print!("\r\n");
                            disable_raw_mode().ok();
                            enable_raw_mode().ok();
                            state.render_prompt();
                        }
                        _ => (),
                    }
                }

                match crossterm::event::poll(Duration::from_millis(0)) {
                    Ok(true) => (),
                    _ => break,
                }
            },
            _ => {
                if let Ok(_) = rx.try_recv() {
                    started_at = Some(std::time::SystemTime::now());
                    // self.handle_event(ev);
                }
            }
        }
    }

    // execute!(stdout, terminal::LeaveAlternateScreen).unwrap();

    terminal::disable_raw_mode().unwrap();
}
