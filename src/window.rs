use std::rc::Rc;

use adw::prelude::*;
use vte::prelude::*;

use crate::terminal::{TerminalSession, TerminalSessionId};

struct WindowState {
    session: Rc<TerminalSession>,
}

impl WindowState {
    fn new(session: Rc<TerminalSession>) -> Self {
        Self { session }
    }

    fn terminal(&self) -> vte::Terminal {
        self.session.widget().clone()
    }
}

pub fn present(app: &adw::Application) {
    let session = Rc::new(TerminalSession::new());
    let session_id: TerminalSessionId = session.id();
    let state = Rc::new(WindowState::new(session));
    let terminal = state.terminal();
    let subtitle = format!("Session {}", session_id.as_u64());

    let header_bar = adw::HeaderBar::builder()
        .title_widget(&adw::WindowTitle::new("Tessera", &subtitle))
        .show_end_title_buttons(true)
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&header_bar);
    content.append(&terminal);

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Tessera")
        .default_width(960)
        .default_height(640)
        .content(&content)
        .build();

    install_terminal_actions(app, &window, &terminal);
    attach_window_state(&window, state);

    window.present();
    terminal.grab_focus();
}

fn install_terminal_actions(
    app: &adw::Application,
    window: &adw::ApplicationWindow,
    terminal: &vte::Terminal,
) {
    let copy_action = gio::SimpleAction::new("copy", None);
    copy_action.connect_activate({
        let terminal = terminal.clone();
        move |_, _| terminal.copy_clipboard_format(vte::Format::Text)
    });
    window.add_action(&copy_action);
    app.set_accels_for_action("win.copy", &["<Control><Shift>c"]);

    let paste_action = gio::SimpleAction::new("paste", None);
    paste_action.connect_activate({
        let terminal = terminal.clone();
        move |_, _| terminal.paste_clipboard()
    });
    window.add_action(&paste_action);
    app.set_accels_for_action("win.paste", &["<Control><Shift>v"]);
}

fn attach_window_state(window: &adw::ApplicationWindow, state: Rc<WindowState>) {
    // SAFETY: The state is written once, never read back through the type-erased API,
    // and is only attached so it drops with the window.
    unsafe {
        window.set_data("dev.lehs.Tessera.window-state", state);
    }
}
