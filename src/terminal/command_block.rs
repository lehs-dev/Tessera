use std::time::{Duration, SystemTime};

use super::session::TerminalSessionId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommandBlockId(u64);

impl CommandBlockId {
    pub fn as_u64(self) -> u64 {
        self.0
    }

    #[cfg(test)]
    pub(crate) fn for_tests(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandBlock {
    pub id: CommandBlockId,
    pub session_id: TerminalSessionId,
    pub started_at: SystemTime,
    pub ended_at: Option<SystemTime>,
    pub exit_status: Option<i32>,
}

impl CommandBlock {
    pub fn duration(&self) -> Option<Duration> {
        self.ended_at?.duration_since(self.started_at).ok()
    }

    #[allow(dead_code)]
    pub fn is_finished(&self) -> bool {
        self.ended_at.is_some()
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandBlockTracker {
    session_id: TerminalSessionId,
    blocks: Vec<CommandBlock>,
    current_block_id: Option<CommandBlockId>,
    next_block_id: u64,
}

#[allow(dead_code)]
impl CommandBlockTracker {
    pub fn new(session_id: TerminalSessionId) -> Self {
        Self {
            session_id,
            blocks: Vec::new(),
            current_block_id: None,
            next_block_id: 1,
        }
    }

    pub fn command_started(&mut self, now: SystemTime) {
        start_command_block(
            self.session_id,
            &mut self.blocks,
            &mut self.current_block_id,
            &mut self.next_block_id,
            now,
        );
    }

    pub fn command_finished(&mut self, status: Option<i32>, now: SystemTime) {
        finish_command_block(
            self.session_id,
            &mut self.blocks,
            &mut self.current_block_id,
            status,
            now,
        );
    }

    pub fn blocks(&self) -> &[CommandBlock] {
        &self.blocks
    }

    pub fn current_block(&self) -> Option<&CommandBlock> {
        let current_block_id = self.current_block_id?;

        self.blocks
            .iter()
            .find(|block| block.id == current_block_id)
    }
}

pub(crate) fn start_command_block(
    session_id: TerminalSessionId,
    blocks: &mut Vec<CommandBlock>,
    current_block_id: &mut Option<CommandBlockId>,
    next_block_id: &mut u64,
    now: SystemTime,
) {
    if let Some(open_block_id) = *current_block_id {
        // The shell semantic stream should send CommandFinished before the next
        // CommandStart. Close the existing block defensively so block metadata
        // remains internally consistent if events arrive out of order.
        match blocks.iter_mut().find(|block| block.id == open_block_id) {
            Some(block) if block.ended_at.is_none() => {
                block.ended_at = Some(now);
                block.exit_status = None;
                eprintln!(
                    "Terminal session {session_id:?} command block {} closed defensively",
                    open_block_id.as_u64()
                );
            }
            Some(_) => {
                eprintln!(
                    "Terminal session {session_id:?} command block {} was already closed before new start",
                    open_block_id.as_u64()
                );
            }
            None => {
                eprintln!(
                    "Terminal session {session_id:?} current command block {} was missing before new start",
                    open_block_id.as_u64()
                );
            }
        }
    }

    let id = CommandBlockId(*next_block_id);
    *next_block_id += 1;

    blocks.push(CommandBlock {
        id,
        session_id,
        started_at: now,
        ended_at: None,
        exit_status: None,
    });
    *current_block_id = Some(id);

    eprintln!(
        "Terminal session {session_id:?} command block {} started",
        id.as_u64()
    );
}

pub(crate) fn finish_command_block(
    session_id: TerminalSessionId,
    blocks: &mut [CommandBlock],
    current_block_id: &mut Option<CommandBlockId>,
    status: Option<i32>,
    now: SystemTime,
) {
    let Some(block_id) = *current_block_id else {
        eprintln!("Terminal session {session_id:?} command finished without a current block");
        return;
    };

    let Some(block) = blocks.iter_mut().find(|block| block.id == block_id) else {
        eprintln!(
            "Terminal session {session_id:?} current command block {} was missing at finish",
            block_id.as_u64()
        );
        *current_block_id = None;
        return;
    };

    block.ended_at = Some(now);
    block.exit_status = status;
    *current_block_id = None;

    eprintln!(
        "Terminal session {session_id:?} command block {} finished status={status:?} duration={:?}",
        block_id.as_u64(),
        block.duration()
    );
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::CommandBlockTracker;
    use crate::terminal::TerminalSessionId;

    fn session_id() -> TerminalSessionId {
        TerminalSessionId::for_tests(7)
    }

    fn at(seconds: u64) -> std::time::SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    #[test]
    fn start_command_creates_one_open_block() {
        let mut tracker = CommandBlockTracker::new(session_id());

        tracker.command_started(at(10));

        let blocks = tracker.blocks();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].id.as_u64(), 1);
        assert_eq!(blocks[0].session_id, session_id());
        assert_eq!(blocks[0].started_at, at(10));
        assert_eq!(blocks[0].ended_at, None);
        assert_eq!(blocks[0].exit_status, None);
        assert_eq!(tracker.current_block(), Some(&blocks[0]));
    }

    #[test]
    fn finish_command_closes_block_with_status() {
        let mut tracker = CommandBlockTracker::new(session_id());

        tracker.command_started(at(10));
        tracker.command_finished(Some(0), at(13));

        let block = &tracker.blocks()[0];
        assert_eq!(block.ended_at, Some(at(13)));
        assert_eq!(block.exit_status, Some(0));
        assert!(block.is_finished());
        assert_eq!(tracker.current_block(), None);
    }

    #[test]
    fn duration_is_computed() {
        let mut tracker = CommandBlockTracker::new(session_id());

        tracker.command_started(at(10));
        tracker.command_finished(Some(0), at(15));

        assert_eq!(tracker.blocks()[0].duration(), Some(Duration::from_secs(5)));
    }

    #[test]
    fn multiple_commands_create_multiple_blocks() {
        let mut tracker = CommandBlockTracker::new(session_id());

        tracker.command_started(at(10));
        tracker.command_finished(Some(0), at(11));
        tracker.command_started(at(12));
        tracker.command_finished(Some(1), at(14));

        let blocks = tracker.blocks();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].id.as_u64(), 1);
        assert_eq!(blocks[0].exit_status, Some(0));
        assert_eq!(blocks[1].id.as_u64(), 2);
        assert_eq!(blocks[1].exit_status, Some(1));
        assert_eq!(tracker.current_block(), None);
    }

    #[test]
    fn finish_without_start_does_not_panic() {
        let mut tracker = CommandBlockTracker::new(session_id());

        tracker.command_finished(Some(1), at(10));

        assert!(tracker.blocks().is_empty());
        assert_eq!(tracker.current_block(), None);
    }

    #[test]
    fn start_while_previous_block_open_closes_previous_defensively() {
        let mut tracker = CommandBlockTracker::new(session_id());

        tracker.command_started(at(10));
        tracker.command_started(at(12));

        let blocks = tracker.blocks();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].ended_at, Some(at(12)));
        assert_eq!(blocks[0].exit_status, None);
        assert_eq!(blocks[1].ended_at, None);
        assert_eq!(tracker.current_block(), Some(&blocks[1]));
    }
}
