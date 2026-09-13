//! Output owned by one invocation, including parallel search buffers.
use std::io::{self, Write};
use std::sync::Mutex;
use termcolor::{Buffer, ColorChoice, WriteColor};
use uucore::context;

pub type Writer = Box<dyn WriteColor + Send>;

pub fn stdout(color: ColorChoice, line: bool) -> Writer {
    let output: Box<dyn Write + Send> = if line {
        Box::new(io::LineWriter::new(context::io::stdout()))
    } else {
        Box::new(io::BufWriter::new(context::io::stdout()))
    };
    if matches!(color, ColorChoice::Never) {
        Box::new(termcolor::NoColor::new(output))
    } else {
        Box::new(termcolor::Ansi::new(output))
    }
}

pub struct BufferWriter {
    color: ColorChoice,
    separator: Option<Vec<u8>>,
    output: Mutex<(context::io::Stdout, bool)>,
}

impl BufferWriter {
    pub fn new(color: ColorChoice, separator: Option<Vec<u8>>) -> Self {
        Self {
            color,
            separator,
            output: Mutex::new((context::io::stdout(), false)),
        }
    }
    pub fn buffer(&self) -> Buffer {
        if matches!(self.color, ColorChoice::Never) {
            Buffer::no_color()
        } else {
            Buffer::ansi()
        }
    }
    pub fn print(&self, buffer: &Buffer) -> io::Result<()> {
        if buffer.is_empty() {
            return Ok(());
        }
        let mut output = self.output.lock().unwrap();
        if output.1 {
            if let Some(separator) = &self.separator {
                output.0.write_all(separator)?;
                output.0.write_all(b"\n")?;
            }
        }
        output.0.write_all(buffer.as_slice())?;
        output.1 = true;
        Ok(())
    }
}

pub fn walk_path(path: &std::path::Path) -> std::path::PathBuf {
    if path == std::path::Path::new("-") {
        path.into()
    } else {
        context::resolve(path)
    }
}

pub fn readable_stdin() -> bool {
    let context = context::snapshot();
    if context.live_input
        || std::io::IsTerminal::is_terminal(context.stdin.as_ref())
    {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        // Like ripgrep's normal heuristic, unknown input defaults to directory search.
        context.stdin.metadata().is_ok_and(|metadata| {
            let kind = metadata.file_type();
            kind.is_file() || kind.is_fifo() || kind.is_socket()
        })
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        unsafe extern "system" {
            fn GetFileType(handle: *mut std::ffi::c_void) -> u32;
        }
        matches!(unsafe { GetFileType(context.stdin.as_raw_handle()) }, 1 | 3)
    }
}
