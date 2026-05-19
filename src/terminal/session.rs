use std::{
    cell::{Cell, RefCell},
    env,
    fs::File,
    io::{self, BufRead, BufReader},
    os::fd::{FromRawFd, OwnedFd, RawFd},
    path::PathBuf,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    thread,
};

use anyhow::{Context, anyhow};
use futures_channel::mpsc::{self, UnboundedSender};
use futures_util::StreamExt;
use gtk::{gdk, prelude::*};
use tessera::shell_integration::event::ShellSemanticEvent;
use vte::prelude::*;

use super::command_block::{finish_command_block, start_command_block};
use super::command_state::apply_semantic_event;
use super::{CommandBlock, CommandBlockId, CommandLifecycleState, TerminalSessionSnapshot};

const ADWAITA_LIGHT_BACKGROUND: gdk::RGBA = gdk::RGBA::new(1.0, 1.0, 1.0, 1.0);
const ADWAITA_LIGHT_FOREGROUND: gdk::RGBA = gdk::RGBA::new(0.0, 0.0, 6.0 / 255.0, 1.0);
const ADWAITA_DARK_BACKGROUND: gdk::RGBA =
    gdk::RGBA::new(30.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0);
const ADWAITA_DARK_FOREGROUND: gdk::RGBA = gdk::RGBA::new(1.0, 1.0, 1.0, 1.0);
const PROXY_EVENT_FD: RawFd = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalBackend {
    DirectShell,
    PtyProxy,
}

impl TerminalBackend {
    fn from_env() -> Self {
        match env::var("TESSERA_TERMINAL_BACKEND") {
            Ok(value) if value == "proxy" => Self::PtyProxy,
            Ok(value) if value.is_empty() || value == "direct" => Self::DirectShell,
            Ok(value) => {
                eprintln!(
                    "Unsupported TESSERA_TERMINAL_BACKEND={value:?}; using direct shell backend"
                );
                Self::DirectShell
            }
            Err(env::VarError::NotPresent) => Self::DirectShell,
            Err(env::VarError::NotUnicode(_)) => {
                eprintln!(
                    "TESSERA_TERMINAL_BACKEND must be valid Unicode; using direct shell backend"
                );
                Self::DirectShell
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TerminalSessionId(u64);

impl TerminalSessionId {
    pub fn as_u64(self) -> u64 {
        self.0
    }

    #[cfg(test)]
    pub(crate) fn for_tests(value: u64) -> Self {
        Self(value)
    }
}

pub struct TerminalSession {
    id: TerminalSessionId,
    backend: TerminalBackend,
    _theme_subscription: TerminalThemeSubscription,
    terminal: vte::Terminal,
    child_pid: Rc<Cell<Option<glib::Pid>>>,
    command_state: Rc<RefCell<CommandLifecycleState>>,
    last_exit_status: Rc<Cell<Option<i32>>>,
    command_count: Rc<Cell<u64>>,
    blocks: Rc<RefCell<Vec<CommandBlock>>>,
    current_block_id: Rc<Cell<Option<CommandBlockId>>>,
    next_block_id: Rc<Cell<u64>>,
}

impl TerminalSession {
    pub fn new() -> Self {
        let terminal = vte::Terminal::builder()
            .allow_hyperlink(true)
            .audible_bell(false)
            .scrollback_lines(10_000)
            .build();

        terminal.set_hexpand(true);
        terminal.set_vexpand(true);
        terminal.set_mouse_autohide(true);

        let theme_subscription = TerminalThemeSubscription::new(&terminal);

        let session = Self {
            id: next_terminal_session_id(),
            backend: TerminalBackend::from_env(),
            _theme_subscription: theme_subscription,
            terminal,
            child_pid: Rc::new(Cell::new(None)),
            command_state: Rc::new(RefCell::new(CommandLifecycleState::Idle)),
            last_exit_status: Rc::new(Cell::new(None)),
            command_count: Rc::new(Cell::new(0)),
            blocks: Rc::new(RefCell::new(Vec::new())),
            current_block_id: Rc::new(Cell::new(None)),
            next_block_id: Rc::new(Cell::new(1)),
        };

        session.install_child_exit_handler();

        if let Err(error) = session.spawn_default_shell() {
            eprintln!("Failed to spawn shell: {error:#}");
        }

        session
    }

    pub fn id(&self) -> TerminalSessionId {
        self.id
    }

    pub fn widget(&self) -> &vte::Terminal {
        &self.terminal
    }

    #[allow(dead_code)]
    pub fn snapshot(&self) -> TerminalSessionSnapshot {
        TerminalSessionSnapshot {
            state: *self.command_state.borrow(),
            last_exit_status: self.last_exit_status.get(),
            command_count: self.command_count.get(),
        }
    }

    #[allow(dead_code)]
    pub fn blocks_snapshot(&self) -> Vec<CommandBlock> {
        self.blocks.borrow().clone()
    }

    #[allow(dead_code)]
    pub fn current_block(&self) -> Option<CommandBlock> {
        let current_block_id = self.current_block_id.get()?;

        self.blocks
            .borrow()
            .iter()
            .find(|block| block.id == current_block_id)
            .cloned()
    }

    pub fn connect_exited<F>(&self, callback: F)
    where
        F: Fn(TerminalSessionId, i32) + 'static,
    {
        let id = self.id;

        self.terminal.connect_child_exited(move |_, status| {
            callback(id, status);
        });
    }

    fn spawn_default_shell(&self) -> anyhow::Result<()> {
        match self.backend {
            TerminalBackend::DirectShell => self.spawn_default_shell_direct(),
            TerminalBackend::PtyProxy => self.spawn_default_shell_via_proxy(),
        }
    }

    fn spawn_default_shell_direct(&self) -> anyhow::Result<()> {
        let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let cwd = env::current_dir().context("could not determine working directory")?;
        let cwd = cwd.to_string_lossy().into_owned();

        let envv = child_environment();
        let envv_refs = envv.iter().map(String::as_str).collect::<Vec<_>>();
        let argv = [shell.as_str()];
        let id = self.id;
        let child_pid = Rc::clone(&self.child_pid);

        child_pid.set(None);

        self.terminal.spawn_async(
            vte::PtyFlags::DEFAULT,
            Some(&cwd),
            &argv,
            &envv_refs,
            glib::SpawnFlags::empty(),
            || {},
            -1,
            gio::Cancellable::NONE,
            move |result| match result {
                Ok(pid) => {
                    child_pid.set(Some(pid));
                    eprintln!("Terminal session {id:?} spawned shell with pid {pid:?}");
                }
                Err(error) => {
                    child_pid.set(None);
                    eprintln!("Failed to spawn shell for session {id:?}: {error}");
                }
            },
        );

        Ok(())
    }

    fn spawn_default_shell_via_proxy(&self) -> anyhow::Result<()> {
        let proxy = proxy_binary_path().context("could not determine tessera-pty-proxy path")?;
        let proxy = proxy
            .to_str()
            .ok_or_else(|| anyhow!("proxy path is not valid UTF-8: {}", proxy.display()))?
            .to_string();
        let cwd = env::current_dir().context("could not determine working directory")?;
        let cwd = cwd.to_string_lossy().into_owned();
        let event_pipe = EventPipe::new().context("could not create proxy event pipe")?;
        let event_sender = self.install_proxy_event_handler();

        start_proxy_event_reader(self.id, event_pipe.read_fd, event_sender)
            .context("could not start proxy event reader")?;

        let envv = proxy_child_environment(PROXY_EVENT_FD);
        let envv_refs = envv.iter().map(String::as_str).collect::<Vec<_>>();
        let argv = [proxy.as_str()];
        let id = self.id;
        let child_pid = Rc::clone(&self.child_pid);

        child_pid.set(None);

        // VTE takes ownership of fds passed to spawn_with_fds_async and maps the
        // proxy event pipe write end onto fd 3 in the child. All other extra fds
        // are closed in the child, which keeps the event channel separate from
        // stdout/stderr without leaking the parent write end.
        unsafe {
            self.terminal.spawn_with_fds_async(
                vte::PtyFlags::DEFAULT,
                Some(&cwd),
                &argv,
                &envv_refs,
                vec![event_pipe.write_fd],
                &[PROXY_EVENT_FD],
                glib::SpawnFlags::empty(),
                || {},
                -1,
                gio::Cancellable::NONE,
                move |result| match result {
                    Ok(pid) => {
                        child_pid.set(Some(pid));
                        eprintln!(
                            "Terminal session {id:?} spawned tessera-pty-proxy with pid {pid:?}"
                        );
                    }
                    Err(error) => {
                        child_pid.set(None);
                        eprintln!("Failed to spawn tessera-pty-proxy for session {id:?}: {error}");
                    }
                },
            );
        }

        Ok(())
    }

    fn install_child_exit_handler(&self) {
        let id = self.id;
        let child_pid = Rc::clone(&self.child_pid);

        self.terminal.connect_child_exited(move |_, status| {
            child_pid.set(None);
            eprintln!("Terminal session {id:?} child exited with status {status}");
        });
    }

    fn install_proxy_event_handler(&self) -> UnboundedSender<ShellSemanticEvent> {
        let (sender, mut receiver) = mpsc::unbounded();
        let id = self.id;
        let command_state = Rc::clone(&self.command_state);
        let last_exit_status = Rc::clone(&self.last_exit_status);
        let command_count = Rc::clone(&self.command_count);
        let blocks = Rc::clone(&self.blocks);
        let current_block_id = Rc::clone(&self.current_block_id);
        let next_block_id = Rc::clone(&self.next_block_id);

        glib::MainContext::default().spawn_local(async move {
            while let Some(event) = receiver.next().await {
                let mut snapshot = TerminalSessionSnapshot {
                    state: *command_state.borrow(),
                    last_exit_status: last_exit_status.get(),
                    command_count: command_count.get(),
                };

                apply_semantic_event(&mut snapshot, &event);

                *command_state.borrow_mut() = snapshot.state;
                last_exit_status.set(snapshot.last_exit_status);
                command_count.set(snapshot.command_count);

                match event {
                    ShellSemanticEvent::CommandStart => {
                        let mut blocks = blocks.borrow_mut();
                        let mut current = current_block_id.get();
                        let mut next = next_block_id.get();

                        start_command_block(
                            id,
                            &mut blocks,
                            &mut current,
                            &mut next,
                            std::time::SystemTime::now(),
                        );

                        current_block_id.set(current);
                        next_block_id.set(next);
                    }
                    ShellSemanticEvent::CommandFinished { status } => {
                        let mut blocks = blocks.borrow_mut();
                        let mut current = current_block_id.get();

                        finish_command_block(
                            id,
                            &mut blocks,
                            &mut current,
                            status,
                            std::time::SystemTime::now(),
                        );

                        current_block_id.set(current);
                    }
                    ShellSemanticEvent::PromptStart | ShellSemanticEvent::PromptEnd => {}
                }

                eprintln!(
                    "Terminal session {id:?} semantic event: {event:?}, state: {:?}, command_count: {}",
                    snapshot.state, snapshot.command_count
                );
            }
        });

        sender
    }
}

struct TerminalThemeSubscription {
    style_manager: adw::StyleManager,
    handler_id: Option<glib::SignalHandlerId>,
}

impl TerminalThemeSubscription {
    fn new(terminal: &vte::Terminal) -> Self {
        let style_manager = adw::StyleManager::default();

        apply_adwaita_terminal_colors(terminal, style_manager.is_dark());

        let terminal_weak = terminal.downgrade();
        let handler_id = style_manager.connect_dark_notify(move |style_manager| {
            let Some(terminal) = terminal_weak.upgrade() else {
                return;
            };

            apply_adwaita_terminal_colors(&terminal, style_manager.is_dark());
        });

        Self {
            style_manager,
            handler_id: Some(handler_id),
        }
    }
}

impl Drop for TerminalThemeSubscription {
    fn drop(&mut self) {
        if let Some(handler_id) = self.handler_id.take() {
            self.style_manager.disconnect(handler_id);
        }
    }
}

fn apply_adwaita_terminal_colors(terminal: &vte::Terminal, is_dark: bool) {
    let (foreground, background) = if is_dark {
        (&ADWAITA_DARK_FOREGROUND, &ADWAITA_DARK_BACKGROUND)
    } else {
        (&ADWAITA_LIGHT_FOREGROUND, &ADWAITA_LIGHT_BACKGROUND)
    };

    terminal.set_color_foreground(foreground);
    terminal.set_color_background(background);
}

impl Default for TerminalSession {
    fn default() -> Self {
        Self::new()
    }
}

fn next_terminal_session_id() -> TerminalSessionId {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);

    TerminalSessionId(NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

fn child_environment() -> Vec<String> {
    child_environment_excluding(&["TERM", "TERM_PROGRAM"])
}

fn proxy_child_environment(event_fd: RawFd) -> Vec<String> {
    let mut envv = child_environment_excluding(&["TERM", "TERM_PROGRAM", "TESSERA_EVENT_FD"]);
    envv.push(format!("TESSERA_EVENT_FD={event_fd}"));

    envv
}

fn child_environment_excluding(excluded_keys: &[&str]) -> Vec<String> {
    let mut envv = env::vars()
        .filter(|(key, _)| !excluded_keys.iter().any(|excluded| *excluded == key))
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>();

    envv.push("TERM=xterm-256color".to_string());
    envv.push("TERM_PROGRAM=Tessera".to_string());

    envv
}

fn proxy_binary_path() -> anyhow::Result<PathBuf> {
    if let Some(path) = env::var_os("TESSERA_PTY_PROXY") {
        return Ok(path.into());
    }

    let current = env::current_exe()?;
    let dir = current
        .parent()
        .ok_or_else(|| anyhow!("could not determine current executable directory"))?;

    // Tessera is currently Linux-focused; the proxy binary is expected to be a
    // sibling of the GUI binary in Cargo target directories.
    Ok(dir.join("tessera-pty-proxy"))
}

struct EventPipe {
    read_fd: OwnedFd,
    write_fd: OwnedFd,
}

impl EventPipe {
    fn new() -> io::Result<Self> {
        let mut fds = [0; 2];

        // SAFETY: pipe2 initializes both entries of fds on success. The returned
        // fds are immediately wrapped in OwnedFd so Rust owns and closes them.
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } == -1 {
            return Err(io::Error::last_os_error());
        }

        // SAFETY: pipe2 succeeded, so fds contains two valid, owned descriptors.
        let read_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
        // SAFETY: pipe2 succeeded, so fds contains two valid, owned descriptors.
        let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };

        Ok(Self { read_fd, write_fd })
    }
}

fn start_proxy_event_reader(
    id: TerminalSessionId,
    read_fd: OwnedFd,
    event_sender: UnboundedSender<ShellSemanticEvent>,
) -> io::Result<()> {
    let thread_name = format!("tessera-proxy-events-{}", id.as_u64());

    thread::Builder::new()
        .name(thread_name)
        .spawn(move || read_proxy_events(id, read_fd, event_sender))
        .map(|_| ())
}

fn read_proxy_events(
    id: TerminalSessionId,
    read_fd: OwnedFd,
    event_sender: UnboundedSender<ShellSemanticEvent>,
) {
    let file = File::from(read_fd);
    let mut reader = BufReader::new(file);

    loop {
        let mut line = String::new();

        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                if line.trim().is_empty() {
                    continue;
                }

                match serde_json::from_str::<ShellSemanticEvent>(&line) {
                    Ok(event) => {
                        if event_sender.unbounded_send(event).is_err() {
                            eprintln!("Terminal session {id:?} semantic event receiver closed");
                            break;
                        }
                    }
                    Err(error) => {
                        eprintln!(
                            "Terminal session {id:?} malformed semantic event JSON: {error}: {line:?}"
                        );
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                eprintln!("Terminal session {id:?} event reader failed: {error}");
                break;
            }
        }
    }
}
