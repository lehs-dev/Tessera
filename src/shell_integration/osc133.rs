const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const OSC_INTRODUCER: &[u8] = b"\x1b]";
const ST_TERMINATOR: &[u8] = b"\x1b\\";
const MAX_BUFFER_LEN: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Osc133Event {
    PromptStart,
    PromptEnd,
    CommandStart { command: Option<String> },
    CommandFinished { status: Option<i32> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Osc133Record {
    Output(Vec<u8>),
    Event(Osc133Event),
}

#[derive(Debug, Default)]
pub struct Osc133Parser {
    buffer: Vec<u8>,
    parse_extended_markers: bool,
}

impl Osc133Parser {
    pub fn with_extended_markers() -> Self {
        Self {
            parse_extended_markers: true,
            ..Self::default()
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Vec<Osc133Event> {
        self.push_records(bytes)
            .into_iter()
            .filter_map(|record| match record {
                Osc133Record::Event(event) => Some(event),
                Osc133Record::Output(_) => None,
            })
            .collect()
    }

    pub fn push_records(&mut self, bytes: &[u8]) -> Vec<Osc133Record> {
        self.buffer.extend_from_slice(bytes);

        let mut records = Vec::new();

        loop {
            let Some(introducer_index) = find_subsequence(&self.buffer, OSC_INTRODUCER) else {
                self.drain_non_osc_bytes(&mut records);
                break;
            };

            if introducer_index > 0 {
                records.push(Osc133Record::Output(
                    self.buffer.drain(..introducer_index).collect(),
                ));
            }

            let Some((terminator_index, terminator_len)) =
                find_terminator(&self.buffer, OSC_INTRODUCER.len())
            else {
                self.enforce_buffer_limit(&mut records);
                break;
            };

            let payload = &self.buffer[OSC_INTRODUCER.len()..terminator_index];
            if let Some(event) = parse_payload(payload, self.parse_extended_markers) {
                records.push(Osc133Record::Event(event));
            } else {
                records.push(Osc133Record::Output(
                    self.buffer[..terminator_index + terminator_len].to_vec(),
                ));
            }

            self.buffer.drain(..terminator_index + terminator_len);
        }

        records
    }

    fn enforce_buffer_limit(&mut self, records: &mut Vec<Osc133Record>) {
        if self.buffer.len() > MAX_BUFFER_LEN {
            let drain_len = self.buffer.len() - MAX_BUFFER_LEN;
            records.push(Osc133Record::Output(
                self.buffer.drain(..drain_len).collect(),
            ));
        }
    }

    fn drain_non_osc_bytes(&mut self, records: &mut Vec<Osc133Record>) {
        if self.buffer.is_empty() {
            return;
        }

        if self.buffer.last() == Some(&ESC) {
            if self.buffer.len() > 1 {
                let drain_len = self.buffer.len() - 1;
                records.push(Osc133Record::Output(
                    self.buffer.drain(..drain_len).collect(),
                ));
            }
        } else {
            records.push(Osc133Record::Output(self.buffer.drain(..).collect()));
        }
    }
}

fn parse_payload(payload: &[u8], parse_extended_markers: bool) -> Option<Osc133Event> {
    let marker = payload.strip_prefix(b"133;")?;

    match marker {
        _ if parse_extended_markers && marker.starts_with(b"A;") => Some(Osc133Event::PromptStart),
        b"A" => Some(Osc133Event::PromptStart),
        _ if parse_extended_markers && marker.starts_with(b"B;") => Some(Osc133Event::PromptEnd),
        b"B" => Some(Osc133Event::PromptEnd),
        _ if parse_extended_markers && marker.starts_with(b"C;") => {
            Some(Osc133Event::CommandStart {
                command: parse_cmdline_url(&marker[2..]),
            })
        }
        b"C" => Some(Osc133Event::CommandStart { command: None }),
        b"D" => Some(Osc133Event::CommandFinished { status: None }),
        _ if marker.starts_with(b"D;") => Some(Osc133Event::CommandFinished {
            status: parse_status(&marker[2..], parse_extended_markers),
        }),
        _ => None,
    }
}

fn parse_cmdline_url(params: &[u8]) -> Option<String> {
    params
        .split(|byte| *byte == b';')
        .find_map(|param| param.strip_prefix(b"cmdline_url="))
        .and_then(percent_decode_utf8)
        .filter(|command| !command.is_empty())
}

fn percent_decode_utf8(bytes: &[u8]) -> Option<String> {
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex_value(*bytes.get(index + 1)?)?;
            let low = hex_value(*bytes.get(index + 2)?)?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }

    String::from_utf8(decoded).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn parse_status(bytes: &[u8], parse_extended_markers: bool) -> Option<i32> {
    let bytes = if parse_extended_markers {
        bytes.split(|byte| *byte == b';').next().unwrap_or(bytes)
    } else {
        bytes
    };

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
    use super::{MAX_BUFFER_LEN, Osc133Event, Osc133Parser, Osc133Record};

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
            vec![Osc133Event::CommandStart { command: None }]
        );
    }

    #[test]
    fn parses_extended_command_start_cmdline_url_when_enabled() {
        let mut parser = Osc133Parser::with_extended_markers();

        assert_eq!(
            parser.push(b"\x1b]133;C;cmdline_url=echo%20hello\x07"),
            vec![Osc133Event::CommandStart {
                command: Some("echo hello".to_string()),
            }]
        );
    }

    #[test]
    fn invalid_cmdline_url_encoding_does_not_panic() {
        let mut parser = Osc133Parser::with_extended_markers();

        assert_eq!(
            parser.push(b"\x1b]133;C;cmdline_url=echo%2hello\x07"),
            vec![Osc133Event::CommandStart { command: None }]
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

        assert_eq!(
            parser.push(b";C\x07"),
            vec![Osc133Event::CommandStart { command: None }]
        );
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
    fn does_not_parse_cmdline_url_from_plain_output() {
        let mut parser = Osc133Parser::with_extended_markers();

        assert!(parser.push(b"cmdline_url=echo%20hello\n").is_empty());
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
                Osc133Event::CommandStart { command: None },
                Osc133Event::CommandFinished { status: Some(1) },
            ]
        );
    }

    #[test]
    fn record_stream_preserves_output_order_and_removes_osc133_markers() {
        let mut parser = Osc133Parser::default();

        assert_eq!(
            parser.push_records(b"\x1b]133;C\x07hello\n\x1b]133;D;0\x07"),
            vec![
                Osc133Record::Event(Osc133Event::CommandStart { command: None }),
                Osc133Record::Output(b"hello\n".to_vec()),
                Osc133Record::Event(Osc133Event::CommandFinished { status: Some(0) }),
            ]
        );
    }

    #[test]
    fn record_stream_preserves_non_osc133_sequences_as_output() {
        let mut parser = Osc133Parser::default();

        assert_eq!(
            parser.push_records(b"before\x1b]0;title\x07after"),
            vec![
                Osc133Record::Output(b"before".to_vec()),
                Osc133Record::Output(b"\x1b]0;title\x07".to_vec()),
                Osc133Record::Output(b"after".to_vec()),
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
    fn ignores_extended_markers_by_default() {
        let mut parser = Osc133Parser::default();

        assert!(parser.push(b"\x1b]133;A;click_events=1\x07").is_empty());
        assert!(
            parser
                .push(b"\x1b]133;C;cmdline_url=echo%20hello\x07")
                .is_empty()
        );
    }

    #[test]
    fn parses_extended_markers_when_enabled() {
        let mut parser = Osc133Parser::with_extended_markers();

        assert_eq!(
            parser.push(
                b"\x1b]133;A;click_events=1\x07\x1b]133;C;cmdline_url=echo%20hello\x07\x1b]133;D;7;extra=1\x07"
            ),
            vec![
                Osc133Event::PromptStart,
                Osc133Event::CommandStart {
                    command: Some("echo hello".to_string()),
                },
                Osc133Event::CommandFinished { status: Some(7) },
            ]
        );
    }

    #[test]
    fn large_non_osc_buffer_is_limited_and_does_not_panic() {
        let mut parser = Osc133Parser::default();
        let bytes = vec![b'x'; MAX_BUFFER_LEN * 2];

        assert!(parser.push(&bytes).is_empty());
        assert!(parser.buffer.len() <= MAX_BUFFER_LEN);
    }
}
