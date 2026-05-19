use std::{
    env,
    ffi::CStr,
    fs::{File, OpenOptions},
    io::{self, ErrorKind, Read, Write},
    mem::MaybeUninit,
    os::{
        fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd},
        unix::process::{CommandExt, ExitStatusExt},
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitCode, ExitStatus},
    thread,
};

use anyhow::{Context, Result, anyhow};
use tessera::shell_integration::{
    event::ShellSemanticEvent,
    osc133::{Osc133Parser, Osc133Record},
};

const READ_BUFFER_LEN: usize = 8 * 1024;
const FISH_INTEGRATION_SCRIPT: &str = "tessera.fish";
const SHELL_INTEGRATION_DIR_ENV: &str = "TESSERA_SHELL_INTEGRATION_DIR";
const SHELL_INTEGRATION_ENABLED_ENV: &str = "TESSERA_ENABLE_SHELL_INTEGRATION";
const OUTPUT_CAPTURE_ENABLED_ENV: &str = "TESSERA_ENABLE_OUTPUT_CAPTURE";
const OUTPUT_CAPTURE_LIMIT_ENV: &str = "TESSERA_OUTPUT_CAPTURE_LIMIT";
const DEFAULT_OUTPUT_CAPTURE_LIMIT: usize = 1024 * 1024;

fn main() -> ExitCode {
    match run() {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("tessera-pty-proxy: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let _raw_mode =
        RawModeGuard::enable(libc::STDIN_FILENO).context("could not put stdin into raw mode")?;
    let mut event_sink = EventSink::from_env().context("could not configure event output")?;
    let pty = open_pty().context("could not open PTY")?;

    copy_terminal_size(libc::STDIN_FILENO, pty.master.as_raw_fd());

    let launch = ShellLaunch::from_environment();
    let parse_extended_osc133 = launch.parse_extended_osc133;
    let mut child =
        spawn_shell(&pty.slave_name, &launch).context("could not spawn shell in PTY")?;
    let master = unsafe { File::from_raw_fd(pty.master.into_raw_fd()) };
    let pty_writer = master.try_clone().context("could not clone PTY master")?;
    let _stdin_relay = thread::spawn(move || relay_stdin_to_pty(pty_writer));
    let output_capture =
        OutputCapture::from_env(parse_extended_osc133, event_sink.is_protocol_channel());

    relay_pty_to_stdout(
        master,
        &mut event_sink,
        parse_extended_osc133,
        output_capture,
    )
    .context("could not relay PTY output")?;

    let status = child.wait().context("could not wait for shell process")?;
    Ok(exit_code_from_status(status))
}

struct Pty {
    master: OwnedFd,
    slave_name: String,
}

fn open_pty() -> io::Result<Pty> {
    let master_fd = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) };
    if master_fd == -1 {
        return Err(io::Error::last_os_error());
    }

    let master = unsafe { OwnedFd::from_raw_fd(master_fd) };

    if unsafe { libc::grantpt(master.as_raw_fd()) } == -1 {
        return Err(io::Error::last_os_error());
    }

    if unsafe { libc::unlockpt(master.as_raw_fd()) } == -1 {
        return Err(io::Error::last_os_error());
    }

    let mut slave_name = [0; 128];
    let result = unsafe {
        libc::ptsname_r(
            master.as_raw_fd(),
            slave_name.as_mut_ptr(),
            slave_name.len(),
        )
    };
    if result != 0 {
        return Err(io::Error::from_raw_os_error(result));
    }

    let slave_name = unsafe { CStr::from_ptr(slave_name.as_ptr()) }
        .to_string_lossy()
        .into_owned();

    Ok(Pty { master, slave_name })
}

fn spawn_shell(slave_name: &str, launch: &ShellLaunch) -> Result<Child> {
    let slave = OpenOptions::new()
        .read(true)
        .write(true)
        .open(slave_name)
        .with_context(|| format!("could not open PTY slave {slave_name}"))?;
    let slave_fd = slave.as_raw_fd();
    let mut command = Command::new(&launch.shell);

    command.args(&launch.args);

    unsafe {
        command.pre_exec(move || configure_child_terminal(slave_fd));
    }

    command
        .spawn()
        .with_context(|| format!("could not execute shell {}", launch.shell))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShellLaunch {
    shell: String,
    args: Vec<String>,
    parse_extended_osc133: bool,
}

impl ShellLaunch {
    fn from_environment() -> Self {
        let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        Self::from_shell(shell)
    }

    fn from_shell(shell: String) -> Self {
        if !shell_integration_enabled() {
            eprintln!("tessera-pty-proxy: shell integration disabled");
            return Self::plain(shell);
        }

        if !is_fish_shell(&shell) {
            eprintln!(
                "tessera-pty-proxy: shell integration requested but unsupported shell: {shell}"
            );
            return Self::plain(shell);
        }

        let Some(script_path) = fish_integration_script_path() else {
            return Self::plain(shell);
        };

        eprintln!("tessera-pty-proxy: shell integration enabled for fish");

        Self {
            shell,
            parse_extended_osc133: true,
            args: vec![
                "-C".to_string(),
                format!(
                    "source {}",
                    fish_single_quoted(&script_path.to_string_lossy())
                ),
            ],
        }
    }

    fn plain(shell: String) -> Self {
        Self {
            shell,
            args: Vec::new(),
            parse_extended_osc133: false,
        }
    }
}

fn shell_integration_enabled() -> bool {
    env_flag_enabled(SHELL_INTEGRATION_ENABLED_ENV)
}

fn is_fish_shell(shell: &str) -> bool {
    Path::new(shell)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "fish")
}

fn fish_integration_script_path() -> Option<PathBuf> {
    let candidate = fish_integration_script_candidate().map(make_absolute);
    let candidate = match candidate {
        Ok(candidate) => candidate,
        Err(error) => {
            eprintln!("tessera-pty-proxy: could not locate fish integration script: {error:#}");
            return None;
        }
    };

    match candidate.canonicalize() {
        Ok(path) if path.is_file() => Some(path),
        Ok(path) => {
            eprintln!(
                "tessera-pty-proxy: fish integration script not found: {}",
                path.display()
            );
            None
        }
        Err(_) => {
            eprintln!(
                "tessera-pty-proxy: fish integration script not found: {}",
                candidate.display()
            );
            None
        }
    }
}

fn fish_integration_script_candidate() -> Result<PathBuf> {
    if let Some(dir) = env::var_os(SHELL_INTEGRATION_DIR_ENV) {
        return Ok(PathBuf::from(dir).join(FISH_INTEGRATION_SCRIPT));
    }

    let current_exe = env::current_exe().context("could not determine current executable path")?;

    development_fish_integration_script_path(&current_exe).ok_or_else(|| {
        anyhow!(
            "could not infer repository root from executable path {}",
            current_exe.display()
        )
    })
}

fn development_fish_integration_script_path(current_exe: &Path) -> Option<PathBuf> {
    let repo_root = current_exe.parent()?.parent()?.parent()?;
    Some(
        repo_root
            .join("shell-integration")
            .join(FISH_INTEGRATION_SCRIPT),
    )
}

fn make_absolute(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        return path;
    }

    match env::current_dir() {
        Ok(cwd) => cwd.join(path),
        Err(_) => path,
    }
}

fn fish_single_quoted(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');

    for character in value.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '\'' => quoted.push_str("\\'"),
            _ => quoted.push(character),
        }
    }

    quoted.push('\'');
    quoted
}

fn configure_child_terminal(slave_fd: RawFd) -> io::Result<()> {
    if unsafe { libc::setsid() } == -1 {
        return Err(io::Error::last_os_error());
    }

    if unsafe { libc::ioctl(slave_fd, libc::TIOCSCTTY, 0) } == -1 {
        return Err(io::Error::last_os_error());
    }

    for target_fd in [libc::STDIN_FILENO, libc::STDOUT_FILENO, libc::STDERR_FILENO] {
        if unsafe { libc::dup2(slave_fd, target_fd) } == -1 {
            return Err(io::Error::last_os_error());
        }
    }

    if slave_fd > libc::STDERR_FILENO && unsafe { libc::close(slave_fd) } == -1 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

fn relay_stdin_to_pty(mut pty_writer: File) -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdin = stdin.lock();
    let mut buffer = [0; READ_BUFFER_LEN];

    loop {
        let bytes_read = match stdin.read(&mut buffer) {
            Ok(0) => break,
            Ok(bytes_read) => bytes_read,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };

        if let Err(error) = pty_writer.write_all(&buffer[..bytes_read]) {
            if is_closed_stream(&error) {
                break;
            }

            return Err(error);
        }
    }

    Ok(())
}

fn relay_pty_to_stdout(
    mut pty_reader: File,
    event_sink: &mut EventSink,
    parse_extended_osc133: bool,
    mut output_capture: OutputCapture,
) -> Result<()> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    let mut parser = if parse_extended_osc133 {
        Osc133Parser::with_extended_markers()
    } else {
        Osc133Parser::default()
    };
    let mut buffer = [0; READ_BUFFER_LEN];

    loop {
        let bytes_read = match pty_reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(bytes_read) => bytes_read,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) if is_closed_stream(&error) => break,
            Err(error) => return Err(error).context("could not read from PTY master"),
        };

        let output = &buffer[..bytes_read];

        if let Err(error) = stdout.write_all(output) {
            if error.kind() == ErrorKind::BrokenPipe {
                break;
            }

            return Err(error).context("could not write PTY output to stdout");
        }

        if let Err(error) = stdout.flush() {
            if error.kind() == ErrorKind::BrokenPipe {
                break;
            }

            return Err(error).context("could not flush stdout");
        }

        for record in parser.push_records(output) {
            match record {
                Osc133Record::Output(output) => output_capture
                    .write_output(&output, event_sink)
                    .context("could not write command output capture event")?,
                Osc133Record::Event(event) => {
                    let event = ShellSemanticEvent::from(event);
                    event_sink
                        .write_event(event.clone())
                        .context("could not write semantic shell event")?;
                    output_capture.observe_event(&event);
                }
            }
        }
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OutputCapture {
    enabled: bool,
    limit: usize,
    running: bool,
    captured_bytes: usize,
    truncated: bool,
}

impl OutputCapture {
    fn from_env(shell_integration_active: bool, event_protocol_available: bool) -> Self {
        if !env_flag_enabled(OUTPUT_CAPTURE_ENABLED_ENV) {
            return Self::disabled();
        }

        if !shell_integration_active {
            eprintln!(
                "tessera-pty-proxy: output capture requested but shell integration is inactive"
            );
            return Self::disabled();
        }

        if !event_protocol_available {
            eprintln!("tessera-pty-proxy: output capture requested but TESSERA_EVENT_FD is unset");
            return Self::disabled();
        }

        Self {
            enabled: true,
            limit: output_capture_limit_from_env(),
            running: false,
            captured_bytes: 0,
            truncated: false,
        }
    }

    fn disabled() -> Self {
        Self {
            enabled: false,
            limit: DEFAULT_OUTPUT_CAPTURE_LIMIT,
            running: false,
            captured_bytes: 0,
            truncated: false,
        }
    }

    #[cfg(test)]
    fn enabled_for_tests(limit: usize) -> Self {
        Self {
            enabled: true,
            limit,
            running: false,
            captured_bytes: 0,
            truncated: false,
        }
    }

    fn observe_event(&mut self, event: &ShellSemanticEvent) {
        if !self.enabled {
            return;
        }

        match event {
            ShellSemanticEvent::CommandStart { .. } => {
                self.running = true;
                self.captured_bytes = 0;
                self.truncated = false;
            }
            ShellSemanticEvent::CommandFinished { .. } => {
                self.running = false;
            }
            ShellSemanticEvent::PromptStart
            | ShellSemanticEvent::PromptEnd
            | ShellSemanticEvent::CommandOutputChunk { .. }
            | ShellSemanticEvent::CommandOutputTruncated { .. } => {}
        }
    }

    fn write_output(&mut self, output: &[u8], event_sink: &mut EventSink) -> Result<()> {
        for event in self.capture_output_events(output) {
            event_sink.write_event(event)?;
        }

        Ok(())
    }

    fn capture_output_events(&mut self, output: &[u8]) -> Vec<ShellSemanticEvent> {
        if !self.enabled || !self.running || output.is_empty() || self.truncated {
            return Vec::new();
        }

        let remaining = self.limit.saturating_sub(self.captured_bytes);
        let capture_len = remaining.min(output.len());
        let mut events = Vec::new();

        if capture_len > 0 {
            self.captured_bytes += capture_len;
            events.push(ShellSemanticEvent::command_output_chunk(
                &output[..capture_len],
            ));
        }

        if capture_len < output.len() {
            self.truncated = true;
            events.push(ShellSemanticEvent::CommandOutputTruncated {
                limit_bytes: self.limit_as_u64(),
            });
        }

        events
    }

    fn limit_as_u64(&self) -> u64 {
        self.limit.try_into().unwrap_or(u64::MAX)
    }
}

fn env_flag_enabled(key: &str) -> bool {
    matches!(env::var(key), Ok(value) if value == "1")
}

fn output_capture_limit_from_env() -> usize {
    match env::var(OUTPUT_CAPTURE_LIMIT_ENV) {
        Ok(value) => match value.parse::<usize>() {
            Ok(limit) => limit,
            Err(_) => {
                eprintln!(
                    "tessera-pty-proxy: invalid {OUTPUT_CAPTURE_LIMIT_ENV}={value:?}; using {DEFAULT_OUTPUT_CAPTURE_LIMIT}"
                );
                DEFAULT_OUTPUT_CAPTURE_LIMIT
            }
        },
        Err(env::VarError::NotPresent) => DEFAULT_OUTPUT_CAPTURE_LIMIT,
        Err(env::VarError::NotUnicode(_)) => {
            eprintln!(
                "tessera-pty-proxy: {OUTPUT_CAPTURE_LIMIT_ENV} must be valid Unicode; using {DEFAULT_OUTPUT_CAPTURE_LIMIT}"
            );
            DEFAULT_OUTPUT_CAPTURE_LIMIT
        }
    }
}

enum EventSink {
    Protocol(File),
    DebugStderr,
}

impl EventSink {
    fn is_protocol_channel(&self) -> bool {
        matches!(self, Self::Protocol(_))
    }

    fn from_env() -> Result<Self> {
        let fd = match env::var("TESSERA_EVENT_FD") {
            Ok(fd) => fd,
            Err(env::VarError::NotPresent) => return Ok(Self::DebugStderr),
            Err(env::VarError::NotUnicode(_)) => {
                return Err(anyhow!("TESSERA_EVENT_FD must be valid Unicode"));
            }
        };

        let fd = fd
            .parse::<RawFd>()
            .with_context(|| format!("TESSERA_EVENT_FD must be a file descriptor, got {fd:?}"))?;

        if fd < 0 {
            return Err(anyhow!("TESSERA_EVENT_FD must not be negative"));
        }

        if fd == libc::STDOUT_FILENO {
            return Err(anyhow!(
                "TESSERA_EVENT_FD must not be stdout; event protocol cannot share terminal output"
            ));
        }

        set_close_on_exec(fd).context("could not mark TESSERA_EVENT_FD close-on-exec")?;

        Ok(Self::Protocol(unsafe { File::from_raw_fd(fd) }))
    }

    fn write_event(&mut self, event: ShellSemanticEvent) -> Result<()> {
        match self {
            Self::Protocol(writer) => event.write_json_line(writer).map_err(Into::into),
            Self::DebugStderr => {
                eprintln!("{event:?}");
                Ok(())
            }
        }
    }
}

fn set_close_on_exec(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }

    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } == -1 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

fn is_closed_stream(error: &io::Error) -> bool {
    error.kind() == ErrorKind::BrokenPipe || error.raw_os_error() == Some(libc::EIO)
}

fn copy_terminal_size(from_fd: RawFd, to_fd: RawFd) {
    let mut winsize = MaybeUninit::<libc::winsize>::uninit();
    let result = unsafe { libc::ioctl(from_fd, libc::TIOCGWINSZ, winsize.as_mut_ptr()) };
    if result == -1 {
        return;
    }

    let winsize = unsafe { winsize.assume_init() };
    unsafe {
        libc::ioctl(to_fd, libc::TIOCSWINSZ, &winsize);
    }
}

struct RawModeGuard {
    fd: RawFd,
    original: Option<libc::termios>,
}

impl RawModeGuard {
    fn enable(fd: RawFd) -> io::Result<Self> {
        if unsafe { libc::isatty(fd) } == 0 {
            return Ok(Self { fd, original: None });
        }

        let mut original = MaybeUninit::<libc::termios>::uninit();
        if unsafe { libc::tcgetattr(fd, original.as_mut_ptr()) } == -1 {
            return Err(io::Error::last_os_error());
        }

        let original = unsafe { original.assume_init() };
        let mut raw = original;
        unsafe {
            libc::cfmakeraw(&mut raw);
        }

        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } == -1 {
            return Err(io::Error::last_os_error());
        }

        Ok(Self {
            fd,
            original: Some(original),
        })
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if let Some(original) = &self.original {
            unsafe {
                libc::tcsetattr(self.fd, libc::TCSANOW, original);
            }
        }
    }
}

fn exit_code_from_status(status: ExitStatus) -> ExitCode {
    if let Some(code) = status.code() {
        return ExitCode::from(code as u8);
    }

    if let Some(signal) = status.signal() {
        return ExitCode::from((128 + signal) as u8);
    }

    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use tessera::shell_integration::event::{ShellSemanticEvent, decode_base64};

    use super::{
        OutputCapture, development_fish_integration_script_path, fish_single_quoted, is_fish_shell,
    };

    #[test]
    fn detects_fish_shell_by_basename() {
        assert!(is_fish_shell("fish"));
        assert!(is_fish_shell("/usr/bin/fish"));
        assert!(is_fish_shell("/bin/fish"));
    }

    #[test]
    fn rejects_non_fish_shells() {
        assert!(!is_fish_shell("/bin/bash"));
        assert!(!is_fish_shell("/usr/bin/zsh"));
        assert!(!is_fish_shell("/usr/bin/fish-wrapper"));
    }

    #[test]
    fn infers_development_script_path_from_debug_binary() {
        let current_exe = Path::new("/repo/target/debug/tessera-pty-proxy");

        let path = development_fish_integration_script_path(current_exe);

        assert_eq!(
            path,
            Some(PathBuf::from("/repo/shell-integration/tessera.fish"))
        );
    }

    #[test]
    fn fish_quotes_source_paths() {
        assert_eq!(
            fish_single_quoted("/tmp/Tessera's path/tessera.fish"),
            "'/tmp/Tessera\\'s path/tessera.fish'"
        );
        assert_eq!(
            fish_single_quoted("/tmp/back\\slash/tessera.fish"),
            "'/tmp/back\\\\slash/tessera.fish'"
        );
    }

    #[test]
    fn output_capture_ignores_output_until_command_is_running() {
        let mut capture = OutputCapture::enabled_for_tests(1024);

        assert!(capture.capture_output_events(b"prompt").is_empty());
    }

    #[test]
    fn output_capture_emits_chunks_while_command_is_running() {
        let mut capture = OutputCapture::enabled_for_tests(1024);
        capture.observe_event(&ShellSemanticEvent::CommandStart { command: None });

        let events = capture.capture_output_events(b"hello\n");

        assert_eq!(events.len(), 1);
        assert_eq!(output_chunk_bytes(&events[0]), b"hello\n");
    }

    #[test]
    fn output_capture_stops_after_command_finished() {
        let mut capture = OutputCapture::enabled_for_tests(1024);
        capture.observe_event(&ShellSemanticEvent::CommandStart { command: None });
        capture.observe_event(&ShellSemanticEvent::CommandFinished { status: Some(0) });

        assert!(capture.capture_output_events(b"next prompt").is_empty());
    }

    #[test]
    fn output_capture_enforces_limit_and_reports_truncation_once() {
        let mut capture = OutputCapture::enabled_for_tests(5);
        capture.observe_event(&ShellSemanticEvent::CommandStart { command: None });

        let first_events = capture.capture_output_events(b"hello");
        let second_events = capture.capture_output_events(b" world");
        let third_events = capture.capture_output_events(b" ignored");

        assert_eq!(first_events.len(), 1);
        assert_eq!(output_chunk_bytes(&first_events[0]), b"hello");
        assert_eq!(
            second_events,
            vec![ShellSemanticEvent::CommandOutputTruncated { limit_bytes: 5 }]
        );
        assert!(third_events.is_empty());
    }

    #[test]
    fn output_capture_truncates_partial_chunk_at_limit() {
        let mut capture = OutputCapture::enabled_for_tests(8);
        capture.observe_event(&ShellSemanticEvent::CommandStart { command: None });

        let events = capture.capture_output_events(b"hello world");

        assert_eq!(events.len(), 2);
        assert_eq!(output_chunk_bytes(&events[0]), b"hello wo");
        assert_eq!(
            events[1],
            ShellSemanticEvent::CommandOutputTruncated { limit_bytes: 8 }
        );
    }

    #[test]
    fn output_capture_resets_limit_for_next_command() {
        let mut capture = OutputCapture::enabled_for_tests(3);
        capture.observe_event(&ShellSemanticEvent::CommandStart { command: None });
        capture.capture_output_events(b"abcd");
        capture.observe_event(&ShellSemanticEvent::CommandFinished { status: Some(0) });
        capture.observe_event(&ShellSemanticEvent::CommandStart { command: None });

        let events = capture.capture_output_events(b"xy");

        assert_eq!(events.len(), 1);
        assert_eq!(output_chunk_bytes(&events[0]), b"xy");
    }

    fn output_chunk_bytes(event: &ShellSemanticEvent) -> Vec<u8> {
        let ShellSemanticEvent::CommandOutputChunk { bytes_base64 } = event else {
            panic!("expected command output chunk, got {event:?}");
        };

        decode_base64(bytes_base64).unwrap()
    }
}
