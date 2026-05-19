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
    pub command: Option<String>,
    pub started_at: SystemTime,
    pub ended_at: Option<SystemTime>,
    pub exit_status: Option<i32>,
    pub output_bytes: Vec<u8>,
    pub output_truncated: bool,
}

impl CommandBlock {
    pub fn duration(&self) -> Option<Duration> {
        self.ended_at?.duration_since(self.started_at).ok()
    }

    #[allow(dead_code)]
    pub fn is_finished(&self) -> bool {
        self.ended_at.is_some()
    }

    pub fn has_output_metadata(&self) -> bool {
        !self.output_bytes.is_empty() || self.output_truncated
    }

    pub fn captured_output_lossy_text(&self) -> String {
        String::from_utf8_lossy(&self.output_bytes).into_owned()
    }

    pub fn to_markdown_with_output(&self) -> String {
        let command = command_text_for_export(self).unwrap_or("<unknown command>");
        let mut output = self.captured_output_lossy_text();

        if self.output_truncated {
            append_output_truncation_note(&mut output);
        }

        format!(
            "## Command #{}\n\n- Status: `{}`\n- Duration: `{}`\n- Output: `{}`\n- Truncated: `{}`\n\n### Command\n\n{}\n\n### Output\n\n{}\n",
            self.id.as_u64(),
            command_block_exit_status_for_markdown(self),
            command_block_duration_for_markdown(self),
            format_command_block_output_size(self.output_bytes.len()),
            self.output_truncated,
            fenced_block("fish", command),
            fenced_block("text", &output),
        )
    }
}

pub(crate) fn format_command_block_markdown(block: &CommandBlock) -> String {
    let mut markdown = format!(
        "## Command Block #{}\n\n- State: {}\n- Exit status: {}\n- Duration: {}\n",
        block.id.as_u64(),
        command_block_state(block),
        command_block_exit_status_for_markdown(block),
        command_block_duration_for_markdown(block),
    );

    match command_text_for_export(block) {
        Some(command) => {
            markdown.push_str("- Command:\n\n");
            markdown.push_str(&fenced_command_block(command));
            markdown.push('\n');
        }
        None => {
            markdown.push_str("- Command: <unknown command>\n");
        }
    }

    if block.has_output_metadata() {
        markdown.push_str(&format!(
            "- Output: {}\n- Output truncated: {}\n",
            format_command_block_output_size(block.output_bytes.len()),
            yes_no(block.output_truncated),
        ));
    }

    markdown
}

pub(crate) fn format_command_blocks_markdown_table(blocks: &[CommandBlock]) -> String {
    if blocks.is_empty() {
        return "_No command blocks._\n".to_string();
    }

    let includes_output_metadata = blocks.iter().any(CommandBlock::has_output_metadata);
    let mut markdown = if includes_output_metadata {
        "| Block | State | Exit status | Duration | Output | Output truncated | Command |\n| --- | --- | --- | --- | --- | --- | --- |\n".to_string()
    } else {
        "| Block | State | Exit status | Duration | Command |\n| --- | --- | --- | --- | --- |\n"
            .to_string()
    };

    for block in blocks {
        if includes_output_metadata {
            markdown.push_str(&format!(
                "| #{} | {} | {} | {} | {} | {} | {} |\n",
                block.id.as_u64(),
                command_block_state(block),
                command_block_exit_status_for_markdown(block),
                command_block_duration_for_markdown(block),
                command_block_output_for_table(block),
                command_block_output_truncated_for_table(block),
                command_block_command_for_table(block),
            ));
        } else {
            markdown.push_str(&format!(
                "| #{} | {} | {} | {} | {} |\n",
                block.id.as_u64(),
                command_block_state(block),
                command_block_exit_status_for_markdown(block),
                command_block_duration_for_markdown(block),
                command_block_command_for_table(block),
            ));
        }
    }

    markdown
}

pub(crate) fn format_command_blocks_markdown_with_output(blocks: &[CommandBlock]) -> String {
    if blocks.is_empty() {
        return "_No command blocks._\n".to_string();
    }

    let mut markdown = String::new();

    for (index, block) in blocks.iter().enumerate() {
        if index > 0 {
            markdown.push('\n');
        }

        markdown.push_str(&block.to_markdown_with_output());
    }

    markdown
}

#[allow(dead_code)]
pub(crate) fn format_command_block_json(block: &CommandBlock) -> serde_json::Result<String> {
    serde_json::to_string_pretty(&CommandBlockExport::from(block))
}

#[allow(dead_code)]
pub(crate) fn format_command_blocks_json(blocks: &[CommandBlock]) -> serde_json::Result<String> {
    let blocks = blocks
        .iter()
        .map(CommandBlockExport::from)
        .collect::<Vec<_>>();

    serde_json::to_string_pretty(&blocks)
}

pub(crate) fn format_command_block_duration(duration: Duration) -> String {
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

pub(crate) fn format_command_block_output_size(bytes: usize) -> String {
    format_byte_size(bytes)
}

fn command_block_state(block: &CommandBlock) -> &'static str {
    if block.ended_at.is_some() {
        "finished"
    } else {
        "running"
    }
}

fn command_block_exit_status_for_markdown(block: &CommandBlock) -> String {
    if block.ended_at.is_none() {
        return "unfinished".to_string();
    }

    match block.exit_status {
        Some(status) => status.to_string(),
        None => "unknown".to_string(),
    }
}

fn command_block_duration_for_markdown(block: &CommandBlock) -> String {
    match block.duration() {
        Some(duration) => format_command_block_duration(duration),
        None if block.ended_at.is_some() => "unknown".to_string(),
        None => "unfinished".to_string(),
    }
}

fn command_block_output_for_table(block: &CommandBlock) -> String {
    if block.has_output_metadata() {
        format_command_block_output_size(block.output_bytes.len())
    } else {
        String::new()
    }
}

fn command_block_output_truncated_for_table(block: &CommandBlock) -> String {
    if block.has_output_metadata() {
        yes_no(block.output_truncated).to_string()
    } else {
        String::new()
    }
}

fn format_byte_size(bytes: usize) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;

    if bytes < 1024 {
        return format!("{bytes} B");
    }

    let bytes = bytes as f64;
    if bytes < MIB {
        return format_decimal_size(bytes / KIB, "KiB");
    }

    format_decimal_size(bytes / MIB, "MiB")
}

fn format_decimal_size(value: f64, unit: &str) -> String {
    let mut formatted = format!("{value:.1}");
    if formatted.ends_with(".0") {
        formatted.truncate(formatted.len() - 2);
    }

    format!("{formatted} {unit}")
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn command_text_for_export(block: &CommandBlock) -> Option<&str> {
    let command = block.command.as_deref()?;

    if command.trim().is_empty() {
        return None;
    }

    Some(command)
}

fn command_block_command_for_table(block: &CommandBlock) -> String {
    command_text_for_export(block)
        .map(escape_markdown_table_cell)
        .unwrap_or_else(|| "<unknown command>".to_string())
}

fn escape_markdown_table_cell(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('\r', "\\r")
        .replace('\n', "<br>")
}

fn fenced_command_block(command: &str) -> String {
    fenced_block("sh", command)
}

fn fenced_block(language: &str, text: &str) -> String {
    let fence = "`".repeat(longest_backtick_run(text).saturating_add(1).max(3));
    let mut block = format!("{fence}{language}\n");
    block.push_str(text);

    if !text.is_empty() && !text.ends_with('\n') {
        block.push('\n');
    }

    block.push_str(&fence);
    block
}

fn append_output_truncation_note(output: &mut String) {
    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }

    output.push_str("[output truncated]");
}

fn longest_backtick_run(text: &str) -> usize {
    let mut longest = 0;
    let mut current = 0;

    for char in text.chars() {
        if char == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }

    longest
}

#[derive(Debug, serde::Serialize)]
struct CommandBlockExport<'a> {
    id: u64,
    session_id: u64,
    command: Option<&'a str>,
    state: &'static str,
    exit_status: Option<i32>,
    duration_ms: Option<u128>,
    #[serde(skip_serializing_if = "is_zero")]
    output_size_bytes: usize,
    #[serde(skip_serializing_if = "is_false")]
    output_truncated: bool,
}

impl<'a> From<&'a CommandBlock> for CommandBlockExport<'a> {
    fn from(block: &'a CommandBlock) -> Self {
        Self {
            id: block.id.as_u64(),
            session_id: block.session_id.as_u64(),
            command: command_text_for_export(block),
            state: command_block_state(block),
            exit_status: block.exit_status,
            duration_ms: block.duration().map(|duration| duration.as_millis()),
            output_size_bytes: block.output_bytes.len(),
            output_truncated: block.output_truncated,
        }
    }
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

fn is_false(value: &bool) -> bool {
    !*value
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

    pub fn command_started(&mut self, command: Option<String>, now: SystemTime) {
        start_command_block(
            self.session_id,
            &mut self.blocks,
            &mut self.current_block_id,
            &mut self.next_block_id,
            command,
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

    pub fn append_output_chunk(&mut self, bytes: &[u8]) {
        append_command_output_chunk(
            self.session_id,
            &mut self.blocks,
            self.current_block_id,
            bytes,
        );
    }

    pub fn mark_output_truncated(&mut self) {
        mark_command_output_truncated(self.session_id, &mut self.blocks, self.current_block_id);
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
    command: Option<String>,
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
        command,
        started_at: now,
        ended_at: None,
        exit_status: None,
        output_bytes: Vec::new(),
        output_truncated: false,
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

pub(crate) fn append_command_output_chunk(
    session_id: TerminalSessionId,
    blocks: &mut [CommandBlock],
    current_block_id: Option<CommandBlockId>,
    bytes: &[u8],
) {
    if bytes.is_empty() {
        return;
    }

    let Some(block_id) = current_block_id else {
        eprintln!(
            "Terminal session {session_id:?} command output chunk received without a current block"
        );
        return;
    };

    let Some(block) = blocks.iter_mut().find(|block| block.id == block_id) else {
        eprintln!(
            "Terminal session {session_id:?} current command block {} was missing for output chunk",
            block_id.as_u64()
        );
        return;
    };

    block.output_bytes.extend_from_slice(bytes);
}

pub(crate) fn mark_command_output_truncated(
    session_id: TerminalSessionId,
    blocks: &mut [CommandBlock],
    current_block_id: Option<CommandBlockId>,
) {
    let Some(block_id) = current_block_id else {
        eprintln!(
            "Terminal session {session_id:?} command output truncation received without a current block"
        );
        return;
    };

    let Some(block) = blocks.iter_mut().find(|block| block.id == block_id) else {
        eprintln!(
            "Terminal session {session_id:?} current command block {} was missing for output truncation",
            block_id.as_u64()
        );
        return;
    };

    block.output_truncated = true;
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::{
        CommandBlock, CommandBlockId, CommandBlockTracker, format_command_block_markdown,
        format_command_blocks_markdown_table,
    };
    use crate::terminal::TerminalSessionId;

    fn session_id() -> TerminalSessionId {
        TerminalSessionId::for_tests(7)
    }

    fn at(seconds: u64) -> std::time::SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
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
            session_id: session_id(),
            command: command.map(str::to_string),
            started_at: at_millis(1_000),
            ended_at,
            exit_status,
            output_bytes: Vec::new(),
            output_truncated: false,
        }
    }

    #[test]
    fn start_command_creates_one_open_block() {
        let mut tracker = CommandBlockTracker::new(session_id());

        tracker.command_started(None, at(10));

        let blocks = tracker.blocks();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].id.as_u64(), 1);
        assert_eq!(blocks[0].session_id, session_id());
        assert_eq!(blocks[0].command.as_deref(), None);
        assert_eq!(blocks[0].started_at, at(10));
        assert_eq!(blocks[0].ended_at, None);
        assert_eq!(blocks[0].exit_status, None);
        assert_eq!(blocks[0].output_bytes, Vec::<u8>::new());
        assert!(!blocks[0].output_truncated);
        assert_eq!(tracker.current_block(), Some(&blocks[0]));
    }

    #[test]
    fn finish_command_closes_block_with_status() {
        let mut tracker = CommandBlockTracker::new(session_id());

        tracker.command_started(None, at(10));
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

        tracker.command_started(None, at(10));
        tracker.command_finished(Some(0), at(15));

        assert_eq!(tracker.blocks()[0].duration(), Some(Duration::from_secs(5)));
    }

    #[test]
    fn multiple_commands_create_multiple_blocks() {
        let mut tracker = CommandBlockTracker::new(session_id());

        tracker.command_started(Some("echo hello".to_string()), at(10));
        tracker.command_finished(Some(0), at(11));
        tracker.command_started(Some("false".to_string()), at(12));
        tracker.command_finished(Some(1), at(14));

        let blocks = tracker.blocks();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].id.as_u64(), 1);
        assert_eq!(blocks[0].command.as_deref(), Some("echo hello"));
        assert_eq!(blocks[0].exit_status, Some(0));
        assert_eq!(blocks[1].id.as_u64(), 2);
        assert_eq!(blocks[1].command.as_deref(), Some("false"));
        assert_eq!(blocks[1].exit_status, Some(1));
        assert_eq!(tracker.current_block(), None);
    }

    #[test]
    fn command_text_is_stored_on_block_start() {
        let mut tracker = CommandBlockTracker::new(session_id());

        tracker.command_started(Some("cargo test".to_string()), at(10));

        assert_eq!(tracker.blocks()[0].command.as_deref(), Some("cargo test"));
        assert_eq!(
            tracker.current_block().unwrap().command.as_deref(),
            Some("cargo test")
        );
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

        tracker.command_started(Some("sleep 1".to_string()), at(10));
        tracker.command_started(Some("echo next".to_string()), at(12));

        let blocks = tracker.blocks();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].command.as_deref(), Some("sleep 1"));
        assert_eq!(blocks[0].ended_at, Some(at(12)));
        assert_eq!(blocks[0].exit_status, None);
        assert_eq!(blocks[1].command.as_deref(), Some("echo next"));
        assert_eq!(blocks[1].ended_at, None);
        assert_eq!(tracker.current_block(), Some(&blocks[1]));
    }

    #[test]
    fn output_chunk_appends_to_current_block() {
        let mut tracker = CommandBlockTracker::new(session_id());

        tracker.command_started(Some("echo hello".to_string()), at(10));
        tracker.append_output_chunk(b"hello");
        tracker.append_output_chunk(b"\n");

        let block = &tracker.blocks()[0];
        assert_eq!(block.output_bytes, b"hello\n");
        assert!(!block.output_truncated);
    }

    #[test]
    fn output_chunk_without_current_block_does_not_panic() {
        let mut tracker = CommandBlockTracker::new(session_id());

        tracker.append_output_chunk(b"orphan output");

        assert!(tracker.blocks().is_empty());
        assert_eq!(tracker.current_block(), None);
    }

    #[test]
    fn output_truncated_marks_current_block() {
        let mut tracker = CommandBlockTracker::new(session_id());

        tracker.command_started(Some("seq 1 1000000".to_string()), at(10));
        tracker.mark_output_truncated();

        assert!(tracker.blocks()[0].output_truncated);
    }

    #[test]
    fn captured_output_lossy_text_preserves_text_and_line_breaks() {
        let mut block = command_block(19, Some("printf hello"), Some(at_millis(1_084)), Some(0));
        block.output_bytes = b"hello\nworld\n".to_vec();
        let original_output = block.output_bytes.clone();

        assert_eq!(block.captured_output_lossy_text(), "hello\nworld\n");
        assert_eq!(block.output_bytes, original_output);
    }

    #[test]
    fn captured_output_lossy_text_is_empty_for_empty_output() {
        let block = command_block(20, Some("true"), Some(at_millis(1_084)), Some(0));

        assert_eq!(block.captured_output_lossy_text(), "");
    }

    #[test]
    fn captured_output_lossy_text_converts_non_utf8_lossily() {
        let mut block = command_block(21, Some("printf bytes"), Some(at_millis(1_084)), Some(0));
        block.output_bytes = vec![b'o', b'k', b' ', 0xff, 0xfe, b'\n'];

        assert_eq!(block.captured_output_lossy_text(), "ok \u{fffd}\u{fffd}\n");
    }

    #[test]
    fn formats_successful_command_block_markdown() {
        let block = command_block(12, Some("echo hello"), Some(at_millis(1_084)), Some(0));

        assert_eq!(
            format_command_block_markdown(&block),
            concat!(
                "## Command Block #12\n\n",
                "- State: finished\n",
                "- Exit status: 0\n",
                "- Duration: 84ms\n",
                "- Command:\n\n",
                "```sh\n",
                "echo hello\n",
                "```\n"
            )
        );
    }

    #[test]
    fn formats_command_block_markdown_with_output_metadata() {
        let mut block = command_block(17, Some("echo hello"), Some(at_millis(1_084)), Some(0));
        block.output_bytes = b"hello\n".to_vec();

        assert_eq!(
            format_command_block_markdown(&block),
            concat!(
                "## Command Block #17\n\n",
                "- State: finished\n",
                "- Exit status: 0\n",
                "- Duration: 84ms\n",
                "- Command:\n\n",
                "```sh\n",
                "echo hello\n",
                "```\n",
                "- Output: 6 B\n",
                "- Output truncated: no\n",
            )
        );
    }

    #[test]
    fn formats_command_block_markdown_with_output_text() {
        let mut block = command_block(22, Some("echo hello"), Some(at_millis(1_084)), Some(0));
        block.output_bytes = b"hello\n".to_vec();

        assert_eq!(
            block.to_markdown_with_output(),
            concat!(
                "## Command #22\n\n",
                "- Status: `0`\n",
                "- Duration: `84ms`\n",
                "- Output: `6 B`\n",
                "- Truncated: `false`\n\n",
                "### Command\n\n",
                "```fish\n",
                "echo hello\n",
                "```\n\n",
                "### Output\n\n",
                "```text\n",
                "hello\n",
                "```\n"
            )
        );
    }

    #[test]
    fn formats_command_block_markdown_with_truncated_output_text() {
        let mut block = command_block(23, Some("yes"), Some(at_millis(1_084)), Some(141));
        block.output_bytes = b"y".to_vec();
        block.output_truncated = true;

        assert_eq!(
            block.to_markdown_with_output(),
            concat!(
                "## Command #23\n\n",
                "- Status: `141`\n",
                "- Duration: `84ms`\n",
                "- Output: `1 B`\n",
                "- Truncated: `true`\n\n",
                "### Command\n\n",
                "```fish\n",
                "yes\n",
                "```\n\n",
                "### Output\n\n",
                "```text\n",
                "y\n",
                "[output truncated]\n",
                "```\n"
            )
        );
    }

    #[test]
    fn formats_command_block_markdown_with_missing_output() {
        let block = command_block(24, Some("true"), Some(at_millis(1_084)), Some(0));

        assert_eq!(
            block.to_markdown_with_output(),
            concat!(
                "## Command #24\n\n",
                "- Status: `0`\n",
                "- Duration: `84ms`\n",
                "- Output: `0 B`\n",
                "- Truncated: `false`\n\n",
                "### Command\n\n",
                "```fish\n",
                "true\n",
                "```\n\n",
                "### Output\n\n",
                "```text\n",
                "```\n"
            )
        );
    }

    #[test]
    fn formats_failed_command_block_markdown() {
        let block = command_block(13, Some("false"), Some(at_millis(1_042)), Some(1));

        assert_eq!(
            format_command_block_markdown(&block),
            concat!(
                "## Command Block #13\n\n",
                "- State: finished\n",
                "- Exit status: 1\n",
                "- Duration: 42ms\n",
                "- Command:\n\n",
                "```sh\n",
                "false\n",
                "```\n"
            )
        );
    }

    #[test]
    fn formats_running_command_block_markdown() {
        let block = command_block(14, Some("sleep 10"), None, None);

        assert_eq!(
            format_command_block_markdown(&block),
            concat!(
                "## Command Block #14\n\n",
                "- State: running\n",
                "- Exit status: unfinished\n",
                "- Duration: unfinished\n",
                "- Command:\n\n",
                "```sh\n",
                "sleep 10\n",
                "```\n"
            )
        );
    }

    #[test]
    fn formats_unknown_command_block_markdown() {
        let block = command_block(15, None, Some(at_millis(1_010)), Some(0));

        assert_eq!(
            format_command_block_markdown(&block),
            concat!(
                "## Command Block #15\n\n",
                "- State: finished\n",
                "- Exit status: 0\n",
                "- Duration: 10ms\n",
                "- Command: <unknown command>\n"
            )
        );
    }

    #[test]
    fn preserves_long_commands_in_command_block_markdown() {
        let command = "printf 'this is a deliberately long command that keeps going past the row limit and should not be truncated in Markdown exports'";
        let block = command_block(16, Some(command), Some(at_millis(1_001)), Some(0));
        let block_markdown = format_command_block_markdown(&block);
        let table_markdown = format_command_blocks_markdown_table(&[block]);

        assert!(block_markdown.contains(command));
        assert!(table_markdown.contains(command));
        assert!(!block_markdown.contains("..."));
        assert!(!table_markdown.contains("..."));
    }

    #[test]
    fn formats_recent_command_blocks_markdown_table() {
        let blocks = [
            command_block(12, Some("echo hello"), Some(at_millis(1_084)), Some(0)),
            command_block(13, Some("sleep 10"), None, None),
        ];

        assert_eq!(
            format_command_blocks_markdown_table(&blocks),
            concat!(
                "| Block | State | Exit status | Duration | Command |\n",
                "| --- | --- | --- | --- | --- |\n",
                "| #12 | finished | 0 | 84ms | echo hello |\n",
                "| #13 | running | unfinished | unfinished | sleep 10 |\n"
            )
        );
    }

    #[test]
    fn formats_recent_command_blocks_markdown_table_with_output_metadata() {
        let mut first = command_block(12, Some("echo hello"), Some(at_millis(1_084)), Some(0));
        first.output_bytes = b"hello\n".to_vec();
        let mut second = command_block(13, Some("seq 1 10000"), Some(at_millis(1_090)), Some(0));
        second.output_bytes = vec![b'x'; 1536];
        second.output_truncated = true;
        let blocks = [first, second];

        assert_eq!(
            format_command_blocks_markdown_table(&blocks),
            concat!(
                "| Block | State | Exit status | Duration | Output | Output truncated | Command |\n",
                "| --- | --- | --- | --- | --- | --- | --- |\n",
                "| #12 | finished | 0 | 84ms | 6 B | no | echo hello |\n",
                "| #13 | finished | 0 | 90ms | 1.5 KiB | yes | seq 1 10000 |\n"
            )
        );
    }
}
