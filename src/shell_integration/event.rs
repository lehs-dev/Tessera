use std::io::{self, Write};

use serde::{Deserialize, Serialize};

use super::osc133::Osc133Event;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ShellSemanticEvent {
    PromptStart,
    PromptEnd,
    CommandStart,
    CommandFinished { status: Option<i32> },
}

impl ShellSemanticEvent {
    pub fn to_json_line(&self) -> serde_json::Result<String> {
        let mut line = serde_json::to_string(self)?;
        line.push('\n');
        Ok(line)
    }

    pub fn write_json_line<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        serde_json::to_writer(&mut *writer, self).map_err(io::Error::other)?;
        writer.write_all(b"\n")?;
        writer.flush()
    }
}

impl From<Osc133Event> for ShellSemanticEvent {
    fn from(event: Osc133Event) -> Self {
        match event {
            Osc133Event::PromptStart => Self::PromptStart,
            Osc133Event::PromptEnd => Self::PromptEnd,
            Osc133Event::CommandStart => Self::CommandStart,
            Osc133Event::CommandFinished { status } => Self::CommandFinished { status },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Osc133Event, ShellSemanticEvent};

    #[test]
    fn serializes_prompt_start() {
        assert_eq!(
            ShellSemanticEvent::PromptStart.to_json_line().unwrap(),
            "{\"event\":\"prompt_start\"}\n"
        );
    }

    #[test]
    fn serializes_command_finished_with_status() {
        assert_eq!(
            ShellSemanticEvent::CommandFinished { status: Some(42) }
                .to_json_line()
                .unwrap(),
            "{\"event\":\"command_finished\",\"status\":42}\n"
        );
    }

    #[test]
    fn serializes_command_finished_without_status() {
        assert_eq!(
            ShellSemanticEvent::CommandFinished { status: None }
                .to_json_line()
                .unwrap(),
            "{\"event\":\"command_finished\",\"status\":null}\n"
        );
    }

    #[test]
    fn converts_from_osc133_event() {
        assert_eq!(
            ShellSemanticEvent::from(Osc133Event::PromptStart),
            ShellSemanticEvent::PromptStart
        );
        assert_eq!(
            ShellSemanticEvent::from(Osc133Event::PromptEnd),
            ShellSemanticEvent::PromptEnd
        );
        assert_eq!(
            ShellSemanticEvent::from(Osc133Event::CommandStart),
            ShellSemanticEvent::CommandStart
        );
        assert_eq!(
            ShellSemanticEvent::from(Osc133Event::CommandFinished { status: Some(7) }),
            ShellSemanticEvent::CommandFinished { status: Some(7) }
        );
    }
}
