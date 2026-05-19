use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use vte::prelude::*;

use crate::terminal::{TerminalSession, TerminalSessionId};

pub struct TerminalWorkspace {
    toolbar_view: adw::ToolbarView,
    tab_view: adw::TabView,
    window_title: adw::WindowTitle,
    sessions: RefCell<HashMap<TerminalSessionId, Rc<TerminalSession>>>,
}

impl TerminalWorkspace {
    pub fn new() -> Rc<Self> {
        let tab_view = adw::TabView::builder().build();
        tab_view.set_hexpand(true);
        tab_view.set_vexpand(true);

        let window_title = adw::WindowTitle::builder().title("Terminal").build();

        let header_bar_new_tab_btn = new_tab_button();
        let header_bar = adw::HeaderBar::builder()
            .show_end_title_buttons(true)
            .title_widget(&window_title)
            .build();
        header_bar.pack_end(&header_bar_new_tab_btn);

        let tab_bar = adw::TabBar::builder()
            .view(&tab_view)
            .autohide(false)
            .expand_tabs(true)
            .build();

        let tab_bar_start_controls = gtk::WindowControls::new(gtk::PackType::Start);
        tab_bar_start_controls.set_valign(gtk::Align::Center);

        let tab_bar_new_tab_btn = new_tab_button();
        let tab_bar_end_controls = gtk::WindowControls::new(gtk::PackType::End);
        tab_bar_end_controls.set_valign(gtk::Align::Center);

        let tab_bar_end_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .valign(gtk::Align::Center)
            .build();
        tab_bar_end_box.append(&tab_bar_new_tab_btn);
        tab_bar_end_box.append(&tab_bar_end_controls);

        tab_bar.set_start_action_widget(Some(&tab_bar_start_controls));
        tab_bar.set_end_action_widget(Some(&tab_bar_end_box));

        let header_stack = gtk::Stack::builder()
            .vhomogeneous(true)
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        header_stack.add_named(&header_bar, Some("title"));
        header_stack.add_named(&tab_bar, Some("tabs"));
        header_stack.set_visible_child(&header_bar);

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header_stack);
        toolbar_view.set_content(Some(&tab_view));

        let workspace = Rc::new(Self {
            toolbar_view,
            tab_view: tab_view.clone(),
            window_title: window_title.clone(),
            sessions: RefCell::new(HashMap::new()),
        });

        let header_stack_weak = header_stack.downgrade();
        let header_bar_weak = header_bar.downgrade();
        let window_title_weak = window_title.downgrade();
        let tab_bar_weak = tab_bar.downgrade();
        tab_view.connect_n_pages_notify(move |tab_view| {
            let (Some(header_stack), Some(header_bar), Some(window_title), Some(tab_bar)) = (
                header_stack_weak.upgrade(),
                header_bar_weak.upgrade(),
                window_title_weak.upgrade(),
                tab_bar_weak.upgrade(),
            ) else {
                return;
            };

            update_window_title(tab_view, &window_title);

            if tab_view.n_pages() > 1 {
                header_stack.set_visible_child(&tab_bar);
            } else {
                header_stack.set_visible_child(&header_bar);
            }
        });

        let window_title_weak = window_title.downgrade();
        tab_view.connect_selected_page_notify(move |tab_view| {
            let Some(window_title) = window_title_weak.upgrade() else {
                return;
            };

            update_window_title(tab_view, &window_title);
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
        update_window_title(&workspace.tab_view, &workspace.window_title);

        workspace
    }

    pub fn widget(&self) -> &adw::ToolbarView {
        &self.toolbar_view
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

        let page = self.tab_view.append(terminal);
        page.set_title("Terminal");

        let page_weak = page.downgrade();
        let tab_view_weak = self.tab_view.downgrade();
        let window_title_weak = self.window_title.downgrade();
        terminal.connect_window_title_notify(move |term| {
            let Some(page) = page_weak.upgrade() else {
                return;
            };

            let title = term.window_title().unwrap_or_default();
            if !title.is_empty() {
                page.set_title(&title);

                let Some(tab_view) = tab_view_weak.upgrade() else {
                    return;
                };
                let Some(window_title) = window_title_weak.upgrade() else {
                    return;
                };

                if tab_view
                    .selected_page()
                    .is_some_and(|selected_page| selected_page == page)
                {
                    window_title.set_title(&title);
                }
            }
        });

        // Workspace reacts to session lifecycle events instead of VTE child signals directly.
        // This keeps the workspace independent from the current session backend.
        let tab_view_weak = self.tab_view.downgrade();
        let page_weak = page.downgrade();
        session.connect_exited(move |_, _| {
            let tab_view = tab_view_weak.clone();
            let page = page_weak.clone();

            glib::idle_add_local_once(move || {
                let (Some(tab_view), Some(page)) = (tab_view.upgrade(), page.upgrade()) else {
                    return;
                };

                tab_view.close_page(&page);
            });
        });

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

fn update_window_title(tab_view: &adw::TabView, window_title: &adw::WindowTitle) {
    let title = tab_view
        .selected_page()
        .map(|page| page.title().to_string())
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| "Terminal".to_string());

    window_title.set_title(&title);
}

fn new_tab_button() -> gtk::Button {
    gtk::Button::builder()
        .icon_name("tab-new-symbolic")
        .tooltip_text("New Tab")
        .action_name("win.new-tab")
        .build()
}
