use adw::prelude::*;
use vte::prelude::*;

use crate::terminal::TerminalSession;

pub fn present(app: &adw::Application) {
    let session = TerminalSession::new();
    let terminal = session.widget().clone();

    let header_bar = adw::HeaderBar::builder()
        .title_widget(&adw::WindowTitle::new("Tessera", "Terminal"))
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
