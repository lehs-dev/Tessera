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
    process::{Child, Command, ExitCode, ExitStatus},
    thread,
};

use anyhow::{Context, Result, anyhow};
use tessera::shell_integration::{event::ShellSemanticEvent, osc133::Osc133Parser};

const READ_BUFFER_LEN: usize = 8 * 1024;

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

    let mut child = spawn_shell(&pty.slave_name).context("could not spawn shell in PTY")?;
    let master = unsafe { File::from_raw_fd(pty.master.into_raw_fd()) };
    let pty_writer = master.try_clone().context("could not clone PTY master")?;
    let _stdin_relay = thread::spawn(move || relay_stdin_to_pty(pty_writer));

    relay_pty_to_stdout(master, &mut event_sink).context("could not relay PTY output")?;

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

fn spawn_shell(slave_name: &str) -> Result<Child> {
    let slave = OpenOptions::new()
        .read(true)
        .write(true)
        .open(slave_name)
        .with_context(|| format!("could not open PTY slave {slave_name}"))?;
    let slave_fd = slave.as_raw_fd();
    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let mut command = Command::new(&shell);

    unsafe {
        command.pre_exec(move || configure_child_terminal(slave_fd));
    }

    command
        .spawn()
        .with_context(|| format!("could not execute shell {shell}"))
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

fn relay_pty_to_stdout(mut pty_reader: File, event_sink: &mut EventSink) -> Result<()> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    let mut parser = Osc133Parser::default();
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

        for event in parser.push(output) {
            event_sink
                .write_event(ShellSemanticEvent::from(event))
                .context("could not write semantic shell event")?;
        }
    }

    Ok(())
}

enum EventSink {
    Protocol(File),
    DebugStderr,
}

impl EventSink {
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
