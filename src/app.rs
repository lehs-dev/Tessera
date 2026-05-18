use adw::prelude::*;

use crate::window;

const APPLICATION_ID: &str = "dev.lehs.Tessera";

pub fn run() {
    let application = adw::Application::builder()
        .application_id(APPLICATION_ID)
        .build();

    install_app_actions(&application);

    application.connect_activate(|app| {
        window::present(app);
    });

    application.run();
}

fn install_app_actions(application: &adw::Application) {
    let quit_action = gio::SimpleAction::new("quit", None);
    quit_action.connect_activate({
        let application = application.downgrade();
        move |_, _| {
            if let Some(application) = application.upgrade() {
                application.quit();
            }
        }
    });

    application.add_action(&quit_action);
    application.set_accels_for_action("app.quit", &["<Control><Shift>q"]);
}
