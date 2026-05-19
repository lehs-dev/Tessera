use std::rc::Rc;

use adw::prelude::*;
use vte::prelude::*;

use crate::workspace::TerminalWorkspace;

pub fn present(app: &adw::Application) {
    let workspace = TerminalWorkspace::new();

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Tessera")
        .default_width(960)
        .default_height(980)
        .content(workspace.widget())
        .build();

    let window_clone = window.clone();
    workspace.tab_view().connect_n_pages_notify(move |tv| {
        if tv.n_pages() == 0 {
            window_clone.close();
        }
    });

    install_window_actions(app, &window, &workspace);
    attach_window_state(&window, Rc::clone(&workspace));

    window.present();
}

fn install_window_actions(
    app: &adw::Application,
    window: &adw::ApplicationWindow,
    workspace: &Rc<TerminalWorkspace>,
) {
    let copy_action = gio::SimpleAction::new("copy", None);
    copy_action.connect_activate({
        let workspace = Rc::clone(workspace);
        move |_, _| {
            if let Some(session) = workspace.active_session() {
                session.widget().copy_clipboard_format(vte::Format::Text);
            }
        }
    });
    window.add_action(&copy_action);
    app.set_accels_for_action("win.copy", &["<Control><Shift>c"]);

    let paste_action = gio::SimpleAction::new("paste", None);
    paste_action.connect_activate({
        let workspace = Rc::clone(workspace);
        move |_, _| {
            if let Some(session) = workspace.active_session() {
                session.widget().paste_clipboard();
            }
        }
    });
    window.add_action(&paste_action);
    app.set_accels_for_action("win.paste", &["<Control><Shift>v"]);

    let new_tab_action = gio::SimpleAction::new("new-tab", None);
    new_tab_action.connect_activate({
        let workspace = Rc::clone(workspace);
        move |_, _| {
            workspace.new_tab();
        }
    });
    window.add_action(&new_tab_action);
    app.set_accels_for_action("win.new-tab", &["<Control><Shift>t"]);

    let close_tab_action = gio::SimpleAction::new("close-tab", None);
    close_tab_action.connect_activate({
        let workspace = Rc::clone(workspace);
        let window = window.clone();
        move |_, _| {
            if workspace.tab_count() > 1 {
                workspace.close_active_tab();
            } else {
                window.close();
            }
        }
    });
    window.add_action(&close_tab_action);
    app.set_accels_for_action("win.close-tab", &["<Control><Shift>w"]);
}

fn attach_window_state(window: &adw::ApplicationWindow, state: Rc<TerminalWorkspace>) {
    // SAFETY: The state is written once, never read back through the type-erased API,
    // and is only attached so it drops with the window.
    unsafe {
        window.set_data("dev.lehs.Tessera.workspace", state);
    }
}
