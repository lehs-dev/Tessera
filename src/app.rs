use adw::prelude::*;
use gtk::gdk;

use crate::window;

const APPLICATION_ID: &str = "dev.lehs.Tessera";

pub fn run() {
    let application = adw::Application::builder()
        .application_id(APPLICATION_ID)
        .build();

    application.connect_startup(|_| {
        load_css();
    });

    install_app_actions(&application);

    application.connect_activate(|app| {
        window::present(app);
    });

    application.run();
}

fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_data(
        "
        tabbar tab {
            min-width: 140px;
            max-width: 220px;
        }
    ",
    );

    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
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
