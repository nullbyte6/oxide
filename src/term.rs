//! Embedded terminal: a shell running on a pseudo-terminal, whose output is parsed into a screen
//! (`vt100`) that the UI draws, and whose input is the keys typed while the terminal is open.

use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

/// Lines of output kept above the visible screen, reachable with Shift+PageUp.
const SCROLLBACK: usize = 5000;

/// One shell running inside a repository.
pub struct Term {
    /// The directory the shell was started in.
    pub repo: PathBuf,
    /// Screen state, fed by the reader thread and read by the UI.
    parser: Arc<Mutex<vt100::Parser>>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    /// Cleared by the reader thread when the shell closes its side (it exited).
    alive: Arc<AtomicBool>,
    /// `(rows, cols)` the pseudo-terminal currently has.
    size: (u16, u16),
}

impl Term {
    /// Starts `$SHELL` (default `/bin/sh`) in `repo` with a screen of `rows` x `cols`.
    pub fn spawn(repo: &Path, rows: u16, cols: u16) -> Result<Term, String> {
        let size = PtySize { rows, cols, pixel_width: 0, pixel_height: 0 };
        let pair = native_pty_system().openpty(size).map_err(|e| format!("cannot open a pseudo-terminal: {e}"))?;

        let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/sh".to_string());
        let mut cmd = CommandBuilder::new(&shell);
        cmd.cwd(repo);
        cmd.env("TERM", "xterm-256color");
        let child = pair.slave.spawn_command(cmd).map_err(|e| format!("cannot run {shell}: {e}"))?;
        drop(pair.slave); // only the shell keeps the slave side, so the reader sees EOF when it exits

        let mut reader = pair.master.try_clone_reader().map_err(|e| format!("cannot read the terminal: {e}"))?;
        let writer = pair.master.take_writer().map_err(|e| format!("cannot write to the terminal: {e}"))?;

        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, SCROLLBACK)));
        let alive = Arc::new(AtomicBool::new(true));
        {
            let (parser, alive) = (Arc::clone(&parser), Arc::clone(&alive));
            thread::spawn(move || {
                let mut buf = [0u8; 8192];
                // Read errors (EIO on Linux once the shell is gone) end the session like EOF does.
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    if let Ok(mut p) = parser.lock() {
                        p.process(&buf[..n]);
                    }
                }
                alive.store(false, Ordering::Relaxed);
            });
        }
        Ok(Term { repo: repo.to_path_buf(), parser, writer, master: pair.master, child, alive, size: (rows, cols) })
    }

    /// The shell is still running.
    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    /// Sends bytes to the shell as if they had been typed.
    pub fn send(&mut self, bytes: &[u8]) {
        if self.writer.write_all(bytes).and_then(|()| self.writer.flush()).is_err() {
            self.alive.store(false, Ordering::Relaxed);
        }
    }

    /// Sends a key press, encoded the way a terminal would.
    pub fn send_key(&mut self, key: KeyEvent) {
        let app_cursor = self.with_screen(|s| s.application_cursor());
        let bytes = key_bytes(key, app_cursor);
        if !bytes.is_empty() {
            self.scroll_to_bottom(); // typing brings the cursor back into view
            self.send(&bytes);
        }
    }

    /// Resizes the pseudo-terminal and the screen; the shell gets SIGWINCH.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        if (rows, cols) == self.size || rows == 0 || cols == 0 {
            return;
        }
        self.size = (rows, cols);
        let _ = self.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
        if let Ok(mut p) = self.parser.lock() {
            p.screen_mut().set_size(rows, cols);
        }
    }

    /// Runs `f` on the current screen.
    pub fn with_screen<R>(&self, f: impl FnOnce(&vt100::Screen) -> R) -> R {
        let p = self.parser.lock().unwrap_or_else(|e| e.into_inner());
        f(p.screen())
    }

    /// Scrolls the view `delta` lines into the history (negative: back towards the present).
    pub fn scroll(&self, delta: isize) {
        let mut p = self.parser.lock().unwrap_or_else(|e| e.into_inner());
        let now = p.screen().scrollback() as isize;
        p.screen_mut().set_scrollback((now + delta).max(0) as usize);
    }

    fn scroll_to_bottom(&self) {
        let mut p = self.parser.lock().unwrap_or_else(|e| e.into_inner());
        p.screen_mut().set_scrollback(0);
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        let _ = self.child.kill(); // closing Oxide or the session must not leave the shell behind
    }
}

/// The bytes a terminal sends for `key`; empty for keys that have none.
fn key_bytes(key: KeyEvent, app_cursor: bool) -> Vec<u8> {
    let KeyEvent { code, modifiers, .. } = key;
    let shift = modifiers.contains(KeyModifiers::SHIFT);
    let alt = modifiers.contains(KeyModifiers::ALT);
    let ctrl = modifiers.contains(KeyModifiers::CONTROL);
    // xterm's modifier parameter: 1 + shift(1) + alt(2) + ctrl(4); 1 means none.
    let param = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);

    // Arrows and Home/End: `ESC [ X`, `ESC O X` in application mode, `ESC [ 1 ; m X` with modifiers.
    let cursor = |letter: char| -> Vec<u8> {
        if param > 1 {
            format!("\x1b[1;{param}{letter}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{letter}").into_bytes()
        } else {
            format!("\x1b[{letter}").into_bytes()
        }
    };
    // Insert, Delete, PageUp, PageDown and F5-F12: `ESC [ n ~`, with the modifier after a `;`.
    let tilde = |n: u8| -> Vec<u8> {
        if param > 1 { format!("\x1b[{n};{param}~").into_bytes() } else { format!("\x1b[{n}~").into_bytes() }
    };

    match code {
        KeyCode::Char(c) => {
            let mut out = Vec::new();
            if alt {
                out.push(0x1b); // Alt is sent as an escape prefix
            }
            if ctrl {
                match c.to_ascii_lowercase() {
                    'a'..='z' => out.push(c.to_ascii_lowercase() as u8 - b'a' + 1),
                    ' ' | '@' | '2' => out.push(0),
                    '[' | '3' => out.push(0x1b),
                    '\\' | '4' => out.push(0x1c),
                    ']' | '5' => out.push(0x1d),
                    '^' | '6' => out.push(0x1e),
                    '_' | '7' | '/' => out.push(0x1f),
                    '8' => out.push(0x7f),
                    _ => return Vec::new(),
                }
            } else {
                let mut b = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
            }
            out
        }
        KeyCode::Enter => b"\r".to_vec(),
        KeyCode::Tab => b"\t".to_vec(),
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => if alt { b"\x1b\x7f".to_vec() } else { b"\x7f".to_vec() },
        KeyCode::Esc => b"\x1b".to_vec(),
        KeyCode::Up => cursor('A'),
        KeyCode::Down => cursor('B'),
        KeyCode::Right => cursor('C'),
        KeyCode::Left => cursor('D'),
        KeyCode::Home => cursor('H'),
        KeyCode::End => cursor('F'),
        KeyCode::Insert => tilde(2),
        KeyCode::Delete => tilde(3),
        KeyCode::PageUp => tilde(5),
        KeyCode::PageDown => tilde(6),
        // F1-F4 are `ESC O P..S`; F5 and up skip numbers (15, 17-21, 23, 24).
        KeyCode::F(n @ 1..=4) => {
            let letter = (b'P' + n - 1) as char;
            if param > 1 { format!("\x1b[1;{param}{letter}").into_bytes() } else { format!("\x1bO{letter}").into_bytes() }
        }
        KeyCode::F(n @ 5..=12) => tilde([15, 17, 18, 19, 20, 21, 23, 24][usize::from(n) - 5]),
        _ => Vec::new(),
    }
}
