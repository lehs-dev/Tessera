use std::env;

use anyhow::Context;
use gtk::prelude::*;
use vte::prelude::*;

pub struct TerminalSession {
    terminal: vte::Terminal,
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

        let session = Self { terminal };
        if let Err(error) = session.spawn_default_shell() {
            eprintln!("Failed to spawn shell: {error:#}");
        }

        session
    }

    pub fn widget(&self) -> &vte::Terminal {
        &self.terminal
    }

    fn spawn_default_shell(&self) -> anyhow::Result<()> {
        let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let cwd = env::current_dir().context("could not determine working directory")?;
        let cwd = cwd.to_string_lossy().into_owned();

        let envv = child_environment();
        let envv_refs = envv.iter().map(String::as_str).collect::<Vec<_>>();
        let argv = [shell.as_str()];

        self.terminal.spawn_async(
            vte::PtyFlags::DEFAULT,
            Some(&cwd),
            &argv,
            &envv_refs,
            glib::SpawnFlags::empty(),
            || {},
            -1,
            gio::Cancellable::NONE,
            |result| {
                if let Err(error) = result {
                    eprintln!("Failed to spawn shell: {error}");
                }
            },
        );

        Ok(())
    }
}

impl Default for TerminalSession {
    fn default() -> Self {
        Self::new()
    }
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
