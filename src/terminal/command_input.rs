use super::CommandLifecycleState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CommandInputAvailability {
    pub can_insert: bool,
    pub can_run: bool,
}

pub(crate) fn can_accept_command_input_state(state: CommandLifecycleState) -> bool {
    !matches!(state, CommandLifecycleState::Running)
}

pub(crate) fn command_input_availability(
    command: Option<&str>,
    state: CommandLifecycleState,
) -> CommandInputAvailability {
    let Some(command) = command.and_then(normalized_command_for_insert) else {
        return CommandInputAvailability {
            can_insert: false,
            can_run: false,
        };
    };

    let can_accept = can_accept_command_input_state(state);

    CommandInputAvailability {
        can_insert: can_accept,
        can_run: can_accept && !is_multiline_command(&command),
    }
}

pub(crate) fn normalized_command_for_insert(command: &str) -> Option<String> {
    let normalized = normalize_command_line_endings(command);

    if normalized.trim().is_empty() {
        return None;
    }

    Some(normalized)
}

pub(crate) fn normalized_command_for_run(command: &str) -> Option<String> {
    let normalized = normalized_command_for_insert(command)?;

    if is_multiline_command(&normalized) {
        return None;
    }

    Some(normalized)
}

fn normalize_command_line_endings(command: &str) -> String {
    command.replace("\r\n", "\n").replace('\r', "\n")
}

fn is_multiline_command(command: &str) -> bool {
    command.contains('\n')
}

#[cfg(test)]
mod tests {
    use super::{
        CommandInputAvailability, command_input_availability, normalized_command_for_insert,
        normalized_command_for_run,
    };
    use crate::terminal::CommandLifecycleState;

    #[test]
    fn empty_command_is_rejected() {
        assert_eq!(normalized_command_for_insert(""), None);
        assert_eq!(normalized_command_for_run(""), None);
        assert_eq!(
            command_input_availability(Some(""), CommandLifecycleState::Prompt),
            CommandInputAvailability {
                can_insert: false,
                can_run: false,
            }
        );
    }

    #[test]
    fn whitespace_only_command_is_rejected() {
        assert_eq!(normalized_command_for_insert(" \t\n "), None);
        assert_eq!(normalized_command_for_run(" \t\n "), None);
        assert_eq!(
            command_input_availability(Some(" \t\n "), CommandLifecycleState::Input),
            CommandInputAvailability {
                can_insert: false,
                can_run: false,
            }
        );
    }

    #[test]
    fn single_line_command_can_run() {
        assert_eq!(
            normalized_command_for_run(" echo hello "),
            Some(" echo hello ".to_string())
        );
        assert_eq!(
            command_input_availability(Some("echo hello"), CommandLifecycleState::Finished),
            CommandInputAvailability {
                can_insert: true,
                can_run: true,
            }
        );
    }

    #[test]
    fn multiline_command_can_insert() {
        assert_eq!(
            normalized_command_for_insert("echo hello\r\ntrue"),
            Some("echo hello\ntrue".to_string())
        );
        assert_eq!(
            command_input_availability(Some("echo hello\ntrue"), CommandLifecycleState::Prompt),
            CommandInputAvailability {
                can_insert: true,
                can_run: false,
            }
        );
    }

    #[test]
    fn multiline_command_cannot_run() {
        assert_eq!(normalized_command_for_run("echo hello\ntrue"), None);
        assert_eq!(normalized_command_for_run("echo hello\rtrue"), None);
    }

    #[test]
    fn running_lifecycle_disables_insert_and_run() {
        assert_eq!(
            command_input_availability(Some("echo hello"), CommandLifecycleState::Running),
            CommandInputAvailability {
                can_insert: false,
                can_run: false,
            }
        );
    }
}
