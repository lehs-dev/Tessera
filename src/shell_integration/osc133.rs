const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const OSC_INTRODUCER: &[u8] = b"\x1b]";
const ST_TERMINATOR: &[u8] = b"\x1b\\";
const MAX_BUFFER_LEN: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Osc133Event {
    PromptStart,
    PromptEnd,
    CommandStart,
    CommandFinished { status: Option<i32> },
}

#[derive(Debug, Default)]
pub struct Osc133Parser {
    buffer: Vec<u8>,
}

impl Osc133Parser {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Osc133Event> {
        self.buffer.extend_from_slice(bytes);
        self.enforce_buffer_limit();

        let mut events = Vec::new();

        loop {
            let Some(introducer_index) = find_subsequence(&self.buffer, OSC_INTRODUCER) else {
                self.discard_non_osc_bytes();
                break;
            };

            if introducer_index > 0 {
                self.buffer.drain(..introducer_index);
            }

            let Some((terminator_index, terminator_len)) =
                find_terminator(&self.buffer, OSC_INTRODUCER.len())
            else {
                break;
            };

            let payload = &self.buffer[OSC_INTRODUCER.len()..terminator_index];
            if let Some(event) = parse_payload(payload) {
                events.push(event);
            }

            self.buffer.drain(..terminator_index + terminator_len);
        }

        events
    }

    fn enforce_buffer_limit(&mut self) {
        if self.buffer.len() > MAX_BUFFER_LEN {
            self.buffer.drain(..self.buffer.len() - MAX_BUFFER_LEN);
        }
    }

    fn discard_non_osc_bytes(&mut self) {
        if self.buffer.last() == Some(&ESC) {
            self.buffer.drain(..self.buffer.len() - 1);
        } else {
            self.buffer.clear();
        }
    }
}

fn parse_payload(payload: &[u8]) -> Option<Osc133Event> {
    let marker = payload.strip_prefix(b"133;")?;

    match marker {
        b"A" => Some(Osc133Event::PromptStart),
        b"B" => Some(Osc133Event::PromptEnd),
        b"C" => Some(Osc133Event::CommandStart),
        b"D" => Some(Osc133Event::CommandFinished { status: None }),
        _ if marker.starts_with(b"D;") => Some(Osc133Event::CommandFinished {
            status: parse_status(&marker[2..]),
        }),
        _ => None,
    }
}

fn parse_status(bytes: &[u8]) -> Option<i32> {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|status| status.parse::<i32>().ok())
}

fn find_terminator(buffer: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut index = start;

    while index < buffer.len() {
        if buffer[index] == BEL {
            return Some((index, 1));
        }

        if buffer[index..].starts_with(ST_TERMINATOR) {
            return Some((index, ST_TERMINATOR.len()));
        }

        index += 1;
    }

    None
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::{MAX_BUFFER_LEN, Osc133Event, Osc133Parser};

    #[test]
    fn parses_prompt_start_bel_terminated() {
        let mut parser = Osc133Parser::default();

        assert_eq!(
            parser.push(b"\x1b]133;A\x07"),
            vec![Osc133Event::PromptStart]
        );
    }

    #[test]
    fn parses_prompt_end_bel_terminated() {
        let mut parser = Osc133Parser::default();

        assert_eq!(parser.push(b"\x1b]133;B\x07"), vec![Osc133Event::PromptEnd]);
    }

    #[test]
    fn parses_command_start_bel_terminated() {
        let mut parser = Osc133Parser::default();

        assert_eq!(
            parser.push(b"\x1b]133;C\x07"),
            vec![Osc133Event::CommandStart]
        );
    }

    #[test]
    fn parses_command_finished_with_status_bel_terminated() {
        let mut parser = Osc133Parser::default();

        assert_eq!(
            parser.push(b"\x1b]133;D;0\x07"),
            vec![Osc133Event::CommandFinished { status: Some(0) }]
        );
    }

    #[test]
    fn parses_command_finished_without_status() {
        let mut parser = Osc133Parser::default();

        assert_eq!(
            parser.push(b"\x1b]133;D\x07"),
            vec![Osc133Event::CommandFinished { status: None }]
        );
    }

    #[test]
    fn parses_st_terminated_sequence() {
        let mut parser = Osc133Parser::default();

        assert_eq!(
            parser.push(b"\x1b]133;A\x1b\\"),
            vec![Osc133Event::PromptStart]
        );
    }

    #[test]
    fn parses_split_sequence_across_push_calls() {
        let mut parser = Osc133Parser::default();

        assert!(parser.push(b"\x1b]133").is_empty());

        assert_eq!(parser.push(b";C\x07"), vec![Osc133Event::CommandStart]);
    }

    #[test]
    fn keeps_incomplete_osc_sequence_buffered_across_push_calls() {
        let mut parser = Osc133Parser::default();

        assert!(parser.push(b"abc\x1b]133;D;").is_empty());

        assert_eq!(
            parser.push(b"42\x07"),
            vec![Osc133Event::CommandFinished { status: Some(42) }]
        );
    }

    #[test]
    fn discards_plain_text_before_osc_marker() {
        let mut parser = Osc133Parser::default();

        assert_eq!(
            parser.push(b"plain text before marker\x1b]133;A\x07"),
            vec![Osc133Event::PromptStart]
        );
    }

    #[test]
    fn ignores_non_osc133_sequences() {
        let mut parser = Osc133Parser::default();

        assert!(parser.push(b"hello world\n").is_empty());
        assert!(parser.push(b"\x1b]0;window title\x07").is_empty());
    }

    #[test]
    fn ignores_non_osc133_sequence_before_osc133_sequence_in_same_chunk() {
        let mut parser = Osc133Parser::default();

        assert_eq!(
            parser.push(b"\x1b]0;window title\x07\x1b]133;B\x07"),
            vec![Osc133Event::PromptEnd]
        );
    }

    #[test]
    fn parses_multiple_events_in_one_chunk() {
        let mut parser = Osc133Parser::default();

        assert_eq!(
            parser.push(b"\x1b]133;A\x07\x1b]133;B\x07\x1b]133;C\x07\x1b]133;D;1\x07"),
            vec![
                Osc133Event::PromptStart,
                Osc133Event::PromptEnd,
                Osc133Event::CommandStart,
                Osc133Event::CommandFinished { status: Some(1) },
            ]
        );
    }

    #[test]
    fn invalid_status_does_not_panic() {
        let mut parser = Osc133Parser::default();

        // Keep the command-finished marker and drop only the malformed status.
        assert_eq!(
            parser.push(b"\x1b]133;D;not-a-status\x07"),
            vec![Osc133Event::CommandFinished { status: None }]
        );
    }

    #[test]
    fn ignores_malformed_osc133_command() {
        let mut parser = Osc133Parser::default();

        assert!(parser.push(b"\x1b]133;X\x07").is_empty());
    }

    #[test]
    fn large_non_osc_buffer_is_limited_and_does_not_panic() {
        let mut parser = Osc133Parser::default();
        let bytes = vec![b'x'; MAX_BUFFER_LEN * 2];

        assert!(parser.push(&bytes).is_empty());
        assert!(parser.buffer.len() <= MAX_BUFFER_LEN);
    }
}
