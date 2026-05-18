use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;

use crate::terminal::{TerminalSession, TerminalSessionId};

pub struct TerminalWorkspace {
    container: gtk::Box,
    tab_view: adw::TabView,
    sessions: RefCell<HashMap<TerminalSessionId, Rc<TerminalSession>>>,
}

impl TerminalWorkspace {
    pub fn new() -> Rc<Self> {
        let tab_view = adw::TabView::builder().build();
        tab_view.set_hexpand(true);
        tab_view.set_vexpand(true);

        let tab_bar = adw::TabBar::builder()
            .view(&tab_view)
            .autohide(true)
            .build();

        let header_bar = adw::HeaderBar::builder()
            .show_end_title_buttons(true)
            .build();

        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.append(&header_bar);
        container.append(&tab_bar);
        container.append(&tab_view);

        let workspace = Rc::new(Self {
            container,
            tab_view: tab_view.clone(),
            sessions: RefCell::new(HashMap::new()),
        });

        // Handle page detach to clean up sessions.
        let workspace_weak = Rc::downgrade(&workspace);
        tab_view.connect_page_detached(move |_, page, _| {
            let Some(workspace) = workspace_weak.upgrade() else {
                return;
            };

            let child = page.child();

            let session_to_remove = {
                let sessions = workspace.sessions.borrow();

                sessions.iter().find_map(|(id, session)| {
                    let widget = session.widget().upcast_ref::<gtk::Widget>();

                    if widget == &child { Some(*id) } else { None }
                })
            };

            if let Some(id) = session_to_remove {
                workspace.sessions.borrow_mut().remove(&id);
            }
        });

        workspace.new_tab();

        workspace
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.container
    }

    pub fn tab_view(&self) -> &adw::TabView {
        &self.tab_view
    }

    pub fn new_tab(&self) -> Rc<TerminalSession> {
        let session = Rc::new(TerminalSession::new());
        let session_id = session.id();

        self.sessions
            .borrow_mut()
            .insert(session_id, Rc::clone(&session));

        let terminal = session.widget();
        let subtitle = format!("Session {}", session_id.as_u64());

        let page = self.tab_view.append(terminal);
        page.set_title(&subtitle);

        self.tab_view.set_selected_page(&page);
        terminal.grab_focus();

        session
    }

    pub fn close_active_tab(&self) {
        if let Some(page) = self.tab_view.selected_page() {
            self.tab_view.close_page(&page);
        }
    }

    pub fn tab_count(&self) -> i32 {
        self.tab_view.n_pages()
    }

    pub fn active_session(&self) -> Option<Rc<TerminalSession>> {
        let page = self.tab_view.selected_page()?;
        let child = page.child();

        for session in self.sessions.borrow().values() {
            if session.widget().upcast_ref::<gtk::Widget>() == &child {
                return Some(Rc::clone(session));
            }
        }

        None
    }

    #[allow(dead_code)]
    pub fn active_session_id(&self) -> Option<TerminalSessionId> {
        self.active_session().map(|s| s.id())
    }
}
