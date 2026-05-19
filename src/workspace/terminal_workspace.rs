use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use glib::object::IsA;
use gtk::{gdk, pango};

use crate::terminal::{
    CommandBlock, CommandLifecycleState, TerminalSession, TerminalSessionId,
    TerminalSessionSnapshot,
};

const COMMAND_DISPLAY_MAX_CHARS: usize = 48;
const COMMAND_BLOCK_DISPLAY_MAX_CHARS: usize = 72;
const RECENT_COMMAND_BLOCK_LIMIT: usize = 20;

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
        let header_bar_recent_blocks_btn = recent_blocks_button();
        let header_bar = adw::HeaderBar::builder()
            .show_end_title_buttons(true)
            .title_widget(&window_title)
            .build();
        header_bar.pack_end(&header_bar_new_tab_btn);
        header_bar.pack_end(&header_bar_recent_blocks_btn);

        let tab_bar = adw::TabBar::builder()
            .view(&tab_view)
            .autohide(false)
            .expand_tabs(true)
            .build();

        let tab_bar_start_controls = gtk::WindowControls::new(gtk::PackType::Start);
        tab_bar_start_controls.set_valign(gtk::Align::Center);

        let tab_bar_new_tab_btn = new_tab_button();
        let tab_bar_recent_blocks_btn = recent_blocks_button();
        let tab_bar_end_controls = gtk::WindowControls::new(gtk::PackType::End);
        tab_bar_end_controls.set_valign(gtk::Align::Center);

        let tab_bar_end_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .valign(gtk::Align::Center)
            .build();
        tab_bar_end_box.append(&tab_bar_recent_blocks_btn);
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
        apply_session_snapshot_to_page(
            &session.snapshot(),
            &page,
            &self.tab_view,
            &self.window_title,
        );

        let page_weak = page.downgrade();
        let tab_view_weak = self.tab_view.downgrade();
        let window_title_weak = self.window_title.downgrade();
        session.connect_state_changed(move |snapshot| {
            let (Some(page), Some(tab_view), Some(window_title)) = (
                page_weak.upgrade(),
                tab_view_weak.upgrade(),
                window_title_weak.upgrade(),
            ) else {
                return;
            };

            apply_session_snapshot_to_page(&snapshot, &page, &tab_view, &window_title);
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

    pub fn show_recent_blocks(&self, parent: &impl IsA<gtk::Window>) {
        let Some(session) = self.active_session() else {
            return;
        };

        let window = gtk::Window::builder()
            .title("Recent Command Blocks")
            .default_width(640)
            .default_height(420)
            .modal(false)
            .transient_for(parent)
            .destroy_with_parent(true)
            .build();

        if let Some(application) = parent.application() {
            window.set_application(Some(&application));
        }

        let title = adw::WindowTitle::builder()
            .title("Recent Command Blocks")
            .subtitle(format!("Session {}", session.id().as_u64()))
            .build();
        let header_bar = adw::HeaderBar::builder()
            .show_end_title_buttons(true)
            .title_widget(&title)
            .build();

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header_bar);
        toolbar_view.set_content(Some(&recent_blocks_content(
            &session.recent_blocks(RECENT_COMMAND_BLOCK_LIMIT),
        )));

        window.set_child(Some(&toolbar_view));
        window.present();
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

fn apply_session_snapshot_to_page(
    snapshot: &TerminalSessionSnapshot,
    page: &adw::TabPage,
    tab_view: &adw::TabView,
    window_title: &adw::WindowTitle,
) {
    let title = format_session_title(snapshot);

    page.set_title(&title);
    page.set_tooltip(&title);

    if tab_view
        .selected_page()
        .as_ref()
        .is_some_and(|selected_page| selected_page == page)
    {
        window_title.set_title(&title);
    }
}

fn format_session_title(snapshot: &TerminalSessionSnapshot) -> String {
    let session = format!("Session {}", snapshot.session_id.as_u64());

    match snapshot.state {
        CommandLifecycleState::Running => {
            let mut parts = vec![session, "Running".to_string()];

            if let Some(block_id) = snapshot.current_block_id {
                parts.push(format!("#{}", block_id.as_u64()));
            }

            if let Some(command) = command_for_display(snapshot.current_block_command.as_deref()) {
                parts.push(command);
            }

            parts.join(" · ")
        }
        CommandLifecycleState::Finished => format_finished_session_title(snapshot),
        CommandLifecycleState::Idle
        | CommandLifecycleState::Prompt
        | CommandLifecycleState::Input => {
            if snapshot.command_count > 0
                && (snapshot.last_exit_status.is_some()
                    || snapshot.last_finished_block_duration.is_some())
            {
                format_finished_session_title(snapshot)
            } else {
                format!("{session} · Idle")
            }
        }
    }
}

fn format_finished_session_title(snapshot: &TerminalSessionSnapshot) -> String {
    let mut parts = vec![
        format!("Session {}", snapshot.session_id.as_u64()),
        match snapshot.last_exit_status {
            Some(status) => format!("exit {status}"),
            None => "exit ?".to_string(),
        },
    ];

    if let Some(duration) = snapshot.last_finished_block_duration {
        parts.push(format_duration(duration));
    }

    if snapshot.command_count > 0 {
        parts.push(format!("#{}", snapshot.command_count));
    }

    if let Some(command) = command_for_display(snapshot.last_finished_block_command.as_deref()) {
        parts.push(command);
    }

    parts.join(" · ")
}

fn command_for_display(command: Option<&str>) -> Option<String> {
    let command = command?.trim();

    if command.is_empty() {
        return None;
    }

    Some(truncate_command_for_display(command))
}

fn truncate_command_for_display(command: &str) -> String {
    truncate_text_for_display(command, COMMAND_DISPLAY_MAX_CHARS)
}

fn truncate_text_for_display(command: &str, max_chars: usize) -> String {
    if command.chars().count() <= max_chars {
        return command.to_string();
    }

    let visible_chars = max_chars.saturating_sub(3);
    let mut display = command.chars().take(visible_chars).collect::<String>();
    display.push_str("...");
    display
}

fn format_duration(duration: Duration) -> String {
    if duration.as_millis() < 1_000 {
        return format!("{}ms", duration.as_millis());
    }

    let seconds = duration.as_secs_f64();
    if seconds < 10.0 {
        let mut formatted = format!("{seconds:.1}");
        if formatted.ends_with(".0") {
            formatted.truncate(formatted.len() - 2);
        }

        return format!("{formatted}s");
    }

    format!("{}s", duration.as_secs())
}

fn new_tab_button() -> gtk::Button {
    gtk::Button::builder()
        .icon_name("tab-new-symbolic")
        .tooltip_text("New Tab")
        .action_name("win.new-tab")
        .build()
}

fn recent_blocks_button() -> gtk::Button {
    gtk::Button::builder()
        .icon_name("view-list-symbolic")
        .tooltip_text("Recent Command Blocks")
        .action_name("win.show-recent-blocks")
        .build()
}

fn recent_blocks_content(blocks: &[CommandBlock]) -> gtk::Widget {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    if blocks.is_empty() {
        let empty_label = gtk::Label::builder()
            .label("No command blocks yet.")
            .xalign(0.0)
            .build();
        content.append(&empty_label);

        return content.upcast();
    }

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .build();
    list.add_css_class("boxed-list");

    for block in blocks {
        list.append(&recent_block_row(block));
    }

    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .child(&list)
        .build();
    content.append(&scrolled);

    content.upcast()
}

fn recent_block_row(block: &CommandBlock) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    let row_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(12)
        .margin_end(12)
        .build();

    let label = gtk::Label::builder()
        .label(format_command_block_row(block))
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(pango::EllipsizeMode::End)
        .build();
    row_box.append(&label);

    if let Some(command) = command_text_for_copy(block) {
        let copy_button = gtk::Button::builder()
            .icon_name("edit-copy-symbolic")
            .tooltip_text("Copy Command")
            .valign(gtk::Align::Center)
            .build();
        let command = command.to_string();
        copy_button.connect_clicked(move |_| copy_command_to_clipboard(&command));
        row_box.append(&copy_button);
    }

    row.set_child(Some(&row_box));
    row
}

fn format_command_block_row(block: &CommandBlock) -> String {
    let mut parts = vec![format!("#{}", block.id.as_u64())];

    if block.ended_at.is_some() {
        parts.push(match block.exit_status {
            Some(status) => format!("exit {status}"),
            None => "exit ?".to_string(),
        });

        if let Some(duration) = block.duration() {
            parts.push(format_duration(duration));
        }
    } else {
        parts.push("running".to_string());
    }

    parts.push(command_block_command_for_display(block.command.as_deref()));

    parts.join(" · ")
}

fn command_block_command_for_display(command: Option<&str>) -> String {
    let command = command
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .unwrap_or("<unknown command>");

    truncate_text_for_display(command, COMMAND_BLOCK_DISPLAY_MAX_CHARS)
}

fn command_text_for_copy(block: &CommandBlock) -> Option<&str> {
    let command = block.command.as_deref()?;

    if command.trim().is_empty() {
        return None;
    }

    Some(command)
}

fn copy_command_to_clipboard(command: &str) {
    let Some(display) = gdk::Display::default() else {
        return;
    };

    display.clipboard().set_text(command);
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use crate::terminal::{
        CommandBlock, CommandBlockId, CommandLifecycleState, TerminalSessionId,
        TerminalSessionSnapshot,
    };

    use super::{format_command_block_row, format_duration, format_session_title};

    fn snapshot(state: CommandLifecycleState) -> TerminalSessionSnapshot {
        TerminalSessionSnapshot {
            session_id: TerminalSessionId::for_tests(3),
            state,
            command_count: 0,
            last_exit_status: None,
            current_block_id: None,
            current_block_command: None,
            last_finished_block_duration: None,
            last_finished_block_command: None,
        }
    }

    fn at_millis(millis: u64) -> std::time::SystemTime {
        UNIX_EPOCH + Duration::from_millis(millis)
    }

    fn command_block(
        id: u64,
        command: Option<&str>,
        ended_at: Option<std::time::SystemTime>,
        exit_status: Option<i32>,
    ) -> CommandBlock {
        CommandBlock {
            id: CommandBlockId::for_tests(id),
            session_id: TerminalSessionId::for_tests(3),
            command: command.map(str::to_string),
            started_at: at_millis(1_000),
            ended_at,
            exit_status,
        }
    }

    #[test]
    fn formats_idle_session() {
        assert_eq!(
            format_session_title(&snapshot(CommandLifecycleState::Idle)),
            "Session 3 · Idle"
        );
    }

    #[test]
    fn formats_running_command() {
        let mut snapshot = snapshot(CommandLifecycleState::Running);
        snapshot.command_count = 11;
        snapshot.current_block_id = Some(CommandBlockId::for_tests(12));

        assert_eq!(format_session_title(&snapshot), "Session 3 · Running · #12");
    }

    #[test]
    fn formats_running_command_with_command_text() {
        let mut snapshot = snapshot(CommandLifecycleState::Running);
        snapshot.command_count = 11;
        snapshot.current_block_id = Some(CommandBlockId::for_tests(12));
        snapshot.current_block_command = Some("echo hello".to_string());

        assert_eq!(
            format_session_title(&snapshot),
            "Session 3 · Running · #12 · echo hello"
        );
    }

    #[test]
    fn formats_finished_command_with_exit_zero() {
        let mut snapshot = snapshot(CommandLifecycleState::Finished);
        snapshot.command_count = 12;
        snapshot.last_exit_status = Some(0);
        snapshot.last_finished_block_duration = Some(Duration::from_millis(84));
        snapshot.last_finished_block_command = Some("echo hello".to_string());

        assert_eq!(
            format_session_title(&snapshot),
            "Session 3 · exit 0 · 84ms · #12 · echo hello"
        );
    }

    #[test]
    fn formats_finished_command_with_exit_one() {
        let mut snapshot = snapshot(CommandLifecycleState::Finished);
        snapshot.command_count = 12;
        snapshot.last_exit_status = Some(1);
        snapshot.last_finished_block_duration = Some(Duration::from_millis(84));

        assert_eq!(
            format_session_title(&snapshot),
            "Session 3 · exit 1 · 84ms · #12"
        );
    }

    #[test]
    fn long_command_is_truncated_in_display() {
        let mut snapshot = snapshot(CommandLifecycleState::Running);
        snapshot.current_block_id = Some(CommandBlockId::for_tests(12));
        snapshot.current_block_command =
            Some("printf 'this is a deliberately long command that keeps going'".to_string());

        assert_eq!(
            format_session_title(&snapshot),
            "Session 3 · Running · #12 · printf 'this is a deliberately long command t..."
        );
    }

    #[test]
    fn missing_command_does_not_break_formatting() {
        let mut snapshot = snapshot(CommandLifecycleState::Finished);
        snapshot.command_count = 12;
        snapshot.last_exit_status = Some(0);
        snapshot.last_finished_block_duration = Some(Duration::from_millis(84));

        assert_eq!(
            format_session_title(&snapshot),
            "Session 3 · exit 0 · 84ms · #12"
        );
    }

    #[test]
    fn formats_duration() {
        assert_eq!(format_duration(Duration::from_millis(84)), "84ms");
        assert_eq!(format_duration(Duration::from_secs(1)), "1s");
        assert_eq!(format_duration(Duration::from_millis(1_500)), "1.5s");
    }

    #[test]
    fn formats_successful_command_block_row() {
        let block = command_block(12, Some("echo hello"), Some(at_millis(1_084)), Some(0));

        assert_eq!(
            format_command_block_row(&block),
            "#12 · exit 0 · 84ms · echo hello"
        );
    }

    #[test]
    fn formats_failed_command_block_row() {
        let block = command_block(13, Some("false"), Some(at_millis(1_042)), Some(1));

        assert_eq!(
            format_command_block_row(&block),
            "#13 · exit 1 · 42ms · false"
        );
    }

    #[test]
    fn formats_running_command_block_row() {
        let block = command_block(14, Some("sleep 10"), None, None);

        assert_eq!(format_command_block_row(&block), "#14 · running · sleep 10");
    }

    #[test]
    fn formats_missing_command_block_row() {
        let block = command_block(15, None, Some(at_millis(1_010)), Some(0));

        assert_eq!(
            format_command_block_row(&block),
            "#15 · exit 0 · 10ms · <unknown command>"
        );
    }

    #[test]
    fn truncates_long_command_block_row_command() {
        let block = command_block(
            16,
            Some(
                "printf 'this is a deliberately long command that keeps going past the row limit'",
            ),
            Some(at_millis(1_001)),
            Some(0),
        );

        assert_eq!(
            format_command_block_row(&block),
            "#16 · exit 0 · 1ms · printf 'this is a deliberately long command that keeps going past the..."
        );
    }
}
