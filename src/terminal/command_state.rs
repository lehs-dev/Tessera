use tessera::shell_integration::event::ShellSemanticEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandLifecycleState {
    Idle,
    Prompt,
    Input,
    Running,
    Finished,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalSessionSnapshot {
    pub state: CommandLifecycleState,
    pub last_exit_status: Option<i32>,
    pub command_count: u64,
}

impl Default for TerminalSessionSnapshot {
    fn default() -> Self {
        Self {
            state: CommandLifecycleState::Idle,
            last_exit_status: None,
            command_count: 0,
        }
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CommandStateTracker {
    snapshot: TerminalSessionSnapshot,
}

#[cfg(test)]
impl CommandStateTracker {
    fn snapshot(&self) -> TerminalSessionSnapshot {
        self.snapshot.clone()
    }

    fn apply_semantic_event(&mut self, event: &ShellSemanticEvent) -> TerminalSessionSnapshot {
        apply_semantic_event(&mut self.snapshot, event);
        self.snapshot()
    }
}

pub fn apply_semantic_event(snapshot: &mut TerminalSessionSnapshot, event: &ShellSemanticEvent) {
    match event {
        ShellSemanticEvent::PromptStart => {
            snapshot.state = CommandLifecycleState::Prompt;
        }
        ShellSemanticEvent::PromptEnd => {
            snapshot.state = CommandLifecycleState::Input;
        }
        ShellSemanticEvent::CommandStart => {
            snapshot.state = CommandLifecycleState::Running;
        }
        ShellSemanticEvent::CommandFinished { status } => {
            snapshot.state = CommandLifecycleState::Finished;
            snapshot.last_exit_status = *status;
            snapshot.command_count += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CommandLifecycleState, CommandStateTracker, TerminalSessionSnapshot, apply_semantic_event,
    };
    use tessera::shell_integration::event::ShellSemanticEvent;

    #[test]
    fn prompt_start_transitions_to_prompt() {
        let mut tracker = CommandStateTracker::default();

        let snapshot = tracker.apply_semantic_event(&ShellSemanticEvent::PromptStart);

        assert_eq!(snapshot.state, CommandLifecycleState::Prompt);
        assert_eq!(snapshot.last_exit_status, None);
        assert_eq!(snapshot.command_count, 0);
    }

    #[test]
    fn prompt_end_transitions_to_input() {
        let mut tracker = CommandStateTracker::default();

        let snapshot = tracker.apply_semantic_event(&ShellSemanticEvent::PromptEnd);

        assert_eq!(snapshot.state, CommandLifecycleState::Input);
        assert_eq!(snapshot.last_exit_status, None);
        assert_eq!(snapshot.command_count, 0);
    }

    #[test]
    fn command_start_transitions_to_running() {
        let mut tracker = CommandStateTracker::default();

        let snapshot = tracker.apply_semantic_event(&ShellSemanticEvent::CommandStart);

        assert_eq!(snapshot.state, CommandLifecycleState::Running);
        assert_eq!(snapshot.last_exit_status, None);
        assert_eq!(snapshot.command_count, 0);
    }

    #[test]
    fn command_finished_with_status_finishes_and_increments_count() {
        let mut tracker = CommandStateTracker::default();

        let snapshot =
            tracker.apply_semantic_event(&ShellSemanticEvent::CommandFinished { status: Some(0) });

        assert_eq!(snapshot.state, CommandLifecycleState::Finished);
        assert_eq!(snapshot.last_exit_status, Some(0));
        assert_eq!(snapshot.command_count, 1);
    }

    #[test]
    fn command_finished_without_status_finishes_and_increments_count() {
        let mut tracker = CommandStateTracker::default();

        let snapshot =
            tracker.apply_semantic_event(&ShellSemanticEvent::CommandFinished { status: None });

        assert_eq!(snapshot.state, CommandLifecycleState::Finished);
        assert_eq!(snapshot.last_exit_status, None);
        assert_eq!(snapshot.command_count, 1);
    }

    #[test]
    fn multiple_command_cycles_increment_count() {
        let mut tracker = CommandStateTracker::default();

        for status in [Some(0), Some(2), None] {
            tracker.apply_semantic_event(&ShellSemanticEvent::PromptStart);
            tracker.apply_semantic_event(&ShellSemanticEvent::PromptEnd);
            tracker.apply_semantic_event(&ShellSemanticEvent::CommandStart);
            tracker.apply_semantic_event(&ShellSemanticEvent::CommandFinished { status });
        }

        assert_eq!(
            tracker.snapshot(),
            TerminalSessionSnapshot {
                state: CommandLifecycleState::Finished,
                last_exit_status: None,
                command_count: 3,
            }
        );
    }

    #[test]
    fn pure_apply_semantic_event_updates_snapshot() {
        let mut snapshot = TerminalSessionSnapshot::default();

        apply_semantic_event(&mut snapshot, &ShellSemanticEvent::PromptStart);

        assert_eq!(snapshot.state, CommandLifecycleState::Prompt);
    }
}
