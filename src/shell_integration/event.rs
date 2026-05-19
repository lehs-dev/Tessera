use std::{
    error::Error,
    fmt,
    io::{self, Write},
};

use serde::{Deserialize, Serialize};

use super::osc133::Osc133Event;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ShellSemanticEvent {
    PromptStart,
    PromptEnd,
    CommandStart {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<String>,
    },
    CommandFinished {
        status: Option<i32>,
    },
    CommandOutputChunk {
        bytes_base64: String,
    },
    CommandOutputTruncated {
        limit_bytes: u64,
    },
}

impl ShellSemanticEvent {
    pub fn command_output_chunk(bytes: &[u8]) -> Self {
        Self::CommandOutputChunk {
            bytes_base64: encode_base64(bytes),
        }
    }

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

pub fn decode_base64(encoded: &str) -> Result<Vec<u8>, Base64DecodeError> {
    let bytes = encoded.as_bytes();

    if !bytes.len().is_multiple_of(4) {
        return Err(Base64DecodeError::new(
            "base64 length must be a multiple of 4",
        ));
    }

    let mut decoded = Vec::with_capacity(bytes.len() / 4 * 3);
    let chunks = bytes.chunks_exact(4);
    let chunk_count = chunks.len();

    for (index, chunk) in chunks.enumerate() {
        let is_last = index + 1 == chunk_count;
        let a = decode_base64_value(chunk[0])?;
        let b = decode_base64_value(chunk[1])?;
        let c = decode_base64_value_or_padding(chunk[2])?;
        let d = decode_base64_value_or_padding(chunk[3])?;

        if matches!(c, Base64Cell::Padding) && !matches!(d, Base64Cell::Padding) {
            return Err(Base64DecodeError::new("invalid base64 padding"));
        }

        if (matches!(c, Base64Cell::Padding) || matches!(d, Base64Cell::Padding)) && !is_last {
            return Err(Base64DecodeError::new(
                "base64 padding must be in the final quartet",
            ));
        }

        decoded.push((a << 2) | (b >> 4));

        let Base64Cell::Value(c) = c else {
            continue;
        };
        decoded.push(((b & 0x0f) << 4) | (c >> 2));

        let Base64Cell::Value(d) = d else {
            continue;
        };
        decoded.push(((c & 0x03) << 6) | d);
    }

    Ok(decoded)
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut chunks = bytes.chunks_exact(3);

    for chunk in &mut chunks {
        encoded.push(TABLE[(chunk[0] >> 2) as usize] as char);
        encoded.push(TABLE[(((chunk[0] & 0x03) << 4) | (chunk[1] >> 4)) as usize] as char);
        encoded.push(TABLE[(((chunk[1] & 0x0f) << 2) | (chunk[2] >> 6)) as usize] as char);
        encoded.push(TABLE[(chunk[2] & 0x3f) as usize] as char);
    }

    match chunks.remainder() {
        [byte] => {
            encoded.push(TABLE[(byte >> 2) as usize] as char);
            encoded.push(TABLE[((byte & 0x03) << 4) as usize] as char);
            encoded.push('=');
            encoded.push('=');
        }
        [first, second] => {
            encoded.push(TABLE[(first >> 2) as usize] as char);
            encoded.push(TABLE[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
            encoded.push(TABLE[((second & 0x0f) << 2) as usize] as char);
            encoded.push('=');
        }
        [] => {}
        _ => unreachable!("chunks_exact(3) remainder is never longer than two bytes"),
    }

    encoded
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Base64DecodeError {
    message: &'static str,
}

impl Base64DecodeError {
    fn new(message: &'static str) -> Self {
        Self { message }
    }
}

impl fmt::Display for Base64DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl Error for Base64DecodeError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Base64Cell {
    Value(u8),
    Padding,
}

fn decode_base64_value(byte: u8) -> Result<u8, Base64DecodeError> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        b'=' => Err(Base64DecodeError::new("unexpected base64 padding")),
        _ => Err(Base64DecodeError::new("invalid base64 character")),
    }
}

fn decode_base64_value_or_padding(byte: u8) -> Result<Base64Cell, Base64DecodeError> {
    if byte == b'=' {
        return Ok(Base64Cell::Padding);
    }

    decode_base64_value(byte).map(Base64Cell::Value)
}

impl From<Osc133Event> for ShellSemanticEvent {
    fn from(event: Osc133Event) -> Self {
        match event {
            Osc133Event::PromptStart => Self::PromptStart,
            Osc133Event::PromptEnd => Self::PromptEnd,
            Osc133Event::CommandStart { command } => Self::CommandStart { command },
            Osc133Event::CommandFinished { status } => Self::CommandFinished { status },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Osc133Event, ShellSemanticEvent, decode_base64};

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
    fn serializes_command_start_without_command() {
        assert_eq!(
            ShellSemanticEvent::CommandStart { command: None }
                .to_json_line()
                .unwrap(),
            "{\"event\":\"command_start\"}\n"
        );
    }

    #[test]
    fn serializes_command_start_with_command() {
        assert_eq!(
            ShellSemanticEvent::CommandStart {
                command: Some("echo hello".to_string()),
            }
            .to_json_line()
            .unwrap(),
            "{\"event\":\"command_start\",\"command\":\"echo hello\"}\n"
        );
    }

    #[test]
    fn deserializes_legacy_command_start_without_command() {
        assert_eq!(
            serde_json::from_str::<ShellSemanticEvent>("{\"event\":\"command_start\"}").unwrap(),
            ShellSemanticEvent::CommandStart { command: None }
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
    fn serializes_command_output_chunk() {
        assert_eq!(
            ShellSemanticEvent::command_output_chunk(b"hello")
                .to_json_line()
                .unwrap(),
            "{\"event\":\"command_output_chunk\",\"bytes_base64\":\"aGVsbG8=\"}\n"
        );
    }

    #[test]
    fn deserializes_command_output_chunk() {
        assert_eq!(
            serde_json::from_str::<ShellSemanticEvent>(
                "{\"event\":\"command_output_chunk\",\"bytes_base64\":\"AAEC/f7/\"}"
            )
            .unwrap(),
            ShellSemanticEvent::CommandOutputChunk {
                bytes_base64: "AAEC/f7/".to_string(),
            }
        );
    }

    #[test]
    fn serializes_command_output_truncated() {
        assert_eq!(
            (ShellSemanticEvent::CommandOutputTruncated {
                limit_bytes: 1_048_576,
            })
            .to_json_line()
            .unwrap(),
            "{\"event\":\"command_output_truncated\",\"limit_bytes\":1048576}\n"
        );
    }

    #[test]
    fn command_output_chunk_preserves_binary_bytes() {
        let event = ShellSemanticEvent::command_output_chunk(&[0, 1, 2, 253, 254, 255]);
        let ShellSemanticEvent::CommandOutputChunk { bytes_base64 } = event else {
            panic!("expected command output chunk");
        };

        assert_eq!(bytes_base64, "AAEC/f7/");
        assert_eq!(
            decode_base64(&bytes_base64).unwrap(),
            vec![0, 1, 2, 253, 254, 255]
        );
    }

    #[test]
    fn rejects_malformed_base64() {
        assert!(decode_base64("aGVsbG8").is_err());
        assert!(decode_base64("aGV!").is_err());
        assert!(decode_base64("a=Vs").is_err());
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
            ShellSemanticEvent::from(Osc133Event::CommandStart {
                command: Some("echo hello".to_string()),
            }),
            ShellSemanticEvent::CommandStart {
                command: Some("echo hello".to_string()),
            }
        );
        assert_eq!(
            ShellSemanticEvent::from(Osc133Event::CommandFinished { status: Some(7) }),
            ShellSemanticEvent::CommandFinished { status: Some(7) }
        );
    }
}
