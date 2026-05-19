use std::{
    cell::Cell,
    env,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::Context;
use gtk::{gdk, prelude::*};
use vte::prelude::*;

const ADWAITA_LIGHT_BACKGROUND: gdk::RGBA = gdk::RGBA::new(1.0, 1.0, 1.0, 1.0);
const ADWAITA_LIGHT_FOREGROUND: gdk::RGBA = gdk::RGBA::new(0.0, 0.0, 6.0 / 255.0, 1.0);
const ADWAITA_DARK_BACKGROUND: gdk::RGBA =
    gdk::RGBA::new(30.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0);
const ADWAITA_DARK_FOREGROUND: gdk::RGBA = gdk::RGBA::new(1.0, 1.0, 1.0, 1.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TerminalSessionId(u64);

impl TerminalSessionId {
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

pub struct TerminalSession {
    id: TerminalSessionId,
    _theme_subscription: TerminalThemeSubscription,
    terminal: vte::Terminal,
    child_pid: Rc<Cell<Option<glib::Pid>>>,
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
            _theme_subscription: theme_subscription,
            terminal,
            child_pid: Rc::new(Cell::new(None)),
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
        spawn_default_shell_for(&self.terminal, self.id, Rc::clone(&self.child_pid))
    }

    fn install_child_exit_handler(&self) {
        let id = self.id;
        let child_pid = Rc::clone(&self.child_pid);

        self.terminal.connect_child_exited(move |_, status| {
            child_pid.set(None);
            eprintln!("Terminal session {id:?} child exited with status {status}");
        });
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

fn spawn_default_shell_for(
    terminal: &vte::Terminal,
    id: TerminalSessionId,
    child_pid: Rc<Cell<Option<glib::Pid>>>,
) -> anyhow::Result<()> {
    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let cwd = env::current_dir().context("could not determine working directory")?;
    let cwd = cwd.to_string_lossy().into_owned();

    let envv = child_environment();
    let envv_refs = envv.iter().map(String::as_str).collect::<Vec<_>>();
    let argv = [shell.as_str()];

    child_pid.set(None);

    terminal.spawn_async(
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

fn child_environment() -> Vec<String> {
    let mut envv = env::vars()
        .filter(|(key, _)| key != "TERM" && key != "TERM_PROGRAM")
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>();

    envv.push("TERM=xterm-256color".to_string());
    envv.push("TERM_PROGRAM=Tessera".to_string());

    envv
}
