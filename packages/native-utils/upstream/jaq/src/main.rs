mod cli;
mod filter;

mod tests;
#[cfg(target_os = "windows")]
mod windows;

use cli::{Cli, Format};
use core::fmt::{self, Display, Formatter};
use filter::run;
use jaq_all::data::{Filter, Runner};
use jaq_all::fmts::read;
use jaq_all::fmts::write::{write, Writer};
use jaq_all::jaq_core::Vars;
use jaq_all::json::write::{Pp, Styles};
use jaq_all::json::Val;
use jaq_all::load::{Color, FileReports, FileReportsDisp, Paint};
use std::path::PathBuf;
use uucore::context::io::{self, Write};
struct ExitCode(i32);
impl ExitCode {
    const SUCCESS: Self = Self(0);
    const FAILURE: Self = Self(1);
    fn from(code: i32) -> Self {
        Self(code)
    }
}
fn with_stdout<T>(out: &mut io::Stdout, f: impl FnOnce(&mut dyn Write) -> T) -> T {
    f(&mut out.lock())
}

extern crate alloc;

pub fn uumain(args: Vec<std::ffi::OsString>) -> i32 {
    main_io(args, &mut io::stdin(), &mut io::stdout(), &mut io::stderr())
        .unwrap_or_else(|error| Error::from(error).report())
        .0
}

fn main_io(
    args: Vec<std::ffi::OsString>,
    inp: &mut io::Stdin,
    out: &mut io::Stdout,
    err: &mut io::Stderr,
) -> io::Result<ExitCode> {
    let cli = match Cli::parse(args) {
        Ok(cli) => cli,
        Err(e) => {
            writeln!(err, "Error: {e}")?;
            return Ok(ExitCode::from(2));
        }
    };

    if cli.version {
        let name = env!("CARGO_PKG_NAME");
        let version = env!("CARGO_PKG_VERSION");
        writeln!(out, "{name} {version}")?;
        Ok(ExitCode::SUCCESS)
    } else if cli.help {
        writeln!(out, "{}", include_str!("help.txt"))?;
        Ok(ExitCode::SUCCESS)
    } else if let Some(test_files) = &cli.run_tests {
        match test_files.last() {
            Some(file) => tests::run(io::BufReader::new(uucore::context::fs::File::open(file)?)),
            None => tests::run(inp.lock()),
        }
    } else {
        real_main(&cli, inp, out).or_else(|e| {
            write!(err, "{}", ErrorColor::new(&e, cli.color_errors()))?;
            Ok(e.report())
        })
    }
}

impl Cli {
    fn runner(&self) -> Runner {
        Runner {
            null_input: self.null_input,
            color_err: self.color_errors(),
            writer: self.writer(),
        }
    }

    fn writer(&self) -> Writer {
        Writer {
            pp: self.pp(),
            format: self.to.unwrap_or_default(),
            join: self.join_output,
        }
    }

    fn pp(&self) -> Pp {
        Pp {
            indent: (!self.compact_output).then(|| self.indent()),
            sort_keys: self.sort_keys,
            styles: self.styles(),
            sep_space: !self.compact_output || matches!(self.to, Some(Format::Yaml)),
        }
    }

    fn styles(&self) -> Styles {
        self.color_output()
            .then(Styles::ansi)
            .map(|c| match uucore::context::env::var("JQ_COLORS") {
                Err(_) => c,
                Ok(s) => c.parse(&s),
            })
            .unwrap_or_default()
    }
}

fn real_main(cli: &Cli, inp: &mut io::Stdin, out: &mut io::Stdout) -> Result<ExitCode, Error> {
    let mut var_val = binds(cli)?;
    let input_filename_idx = var_val.len();
    var_val.push(("!input_filename".to_string(), Val::Null));
    let (var_names, mut vars): (Vec<String>, Vec<Val>) = var_val.into_iter().unzip();

    let (var_vals, filter) = match &cli.filter {
        None => (Vec::new(), Filter::default()),
        Some(filter) => {
            let (path, code) = match filter {
                cli::Filter::FromFile(path) => {
                    (path.into(), uucore::context::fs::read_to_string(path)?)
                }
                cli::Filter::Inline(filter) => ("<inline>".into(), filter.clone()),
            };
            filter::parse_compile(&path, &code, &var_names, &cli.library_path)
                .map_err(Error::Report)?
        }
    };
    vars.extend(var_vals);
    //println!("Filter: {:?}", filter);

    let runner = &cli.runner();
    let writer = &runner.writer;

    let unwrap_or_json = |fmt: Option<Format>| fmt.unwrap_or_default();
    let last = if cli.files.is_empty() {
        vars[input_filename_idx] = Val::utf8_str("<stdin>");
        let vars = Vars::new(vars);

        let format = unwrap_or_json(cli.from);
        let s = read::read_string(format, inp.lock())?;
        let inputs = read::read(format, inp.lock(), &s, cli.slurp);
        with_stdout(out, |out| {
            run(runner, &filter, vars, inputs, |v| write(out, writer, &v))
        })?
    } else {
        let mut last = None;
        for file in &cli.files {
            vars[input_filename_idx] =
                Val::utf8_str(file.as_os_str().to_string_lossy().into_owned());
            let vars = Vars::new(vars.clone());

            let resolved = uucore::context::resolve(file);
            let path = resolved.as_path();
            let bytes = read::load_file(uucore::context::resolve(path))
                .map_err(|e| Error::Io(Some(path.display().to_string()), e))?;
            let format = unwrap_or_json(cli.from.or_else(|| Format::determine(path)));
            let s = read::bytes_str(format, &bytes)?;
            let inputs = read::parse(format, &bytes, s, cli.slurp);

            if cli.in_place {
                // create a temporary file where output is written to
                let location = path.parent().unwrap();
                let mut tmp = tempfile::Builder::new()
                    .prefix("jaq")
                    .tempfile_in(location)?;

                last = run(runner, &filter, vars.clone(), inputs, |output| {
                    write(tmp.as_file_mut(), writer, &output)
                })?;

                // replace the input file with the temporary file
                std::mem::drop(bytes);
                let perms = uucore::context::fs::metadata(path)?.permissions();
                tmp.persist(path).map_err(|e| Error::Io(None, e.into()))?;
                uucore::context::fs::set_permissions(path, perms)?;
            } else {
                last = with_stdout(out, |out| {
                    run(runner, &filter, vars.clone(), inputs, |v| {
                        write(out, writer, &v)
                    })
                })?;
            }
        }
        last
    };

    if cli.exit_status {
        last.map_or_else(
            || Err(Error::NoOutput),
            |b| b.then_some(ExitCode::SUCCESS).ok_or(Error::FalseOrNull),
        )
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn binds(cli: &Cli) -> Result<Vec<(String, Val)>, Error> {
    let arg = cli.arg.iter().map(|(k, s)| {
        let s = s.to_owned();
        Ok((k.to_owned(), Val::utf8_str(s)))
    });
    let argjson = cli.argjson.iter().map(|(k, s)| {
        let err = |e| Error::Parse(format!("{e} (for value passed to `--argjson {k}`)"));
        let s = s.as_bytes();
        Ok((k.to_owned(), read::json::parse_single(s).map_err(err)?))
    });
    let rawfile = cli.rawfile.iter().map(|(k, path)| {
        let err = |e| Error::Io(Some(format!("{path:?}")), e);
        let s = read::load_file(uucore::context::resolve(path)).map_err(err)?;
        Ok((k.to_owned(), Val::utf8_str(s)))
    });
    let slurpfile = cli.slurpfile.iter().map(|(k, path)| {
        let err = |e| Error::Io(Some(format!("{path:?}")), e);
        Ok((
            k.to_owned(),
            read::json_array(uucore::context::resolve(path)).map_err(err)?,
        ))
    });

    let positional = cli.args.iter().cloned().map(|s| Ok(Val::from(s)));
    let positional = positional.collect::<Result<Vec<_>, Error>>()?;

    let var_val = arg.chain(rawfile).chain(slurpfile).chain(argjson);
    let mut var_val = var_val.collect::<Result<Vec<_>, Error>>()?;

    var_val.push(("ARGS".to_string(), args(&positional, &var_val)));
    let env = uucore::context::env::vars().map(|(k, v)| (k.into(), Val::from(v)));
    var_val.push(("ENV".to_string(), Val::obj(env.collect())));

    Ok(var_val)
}

fn args(positional: &[Val], named: &[(String, Val)]) -> Val {
    let key = |k: &str| k.to_string().into();
    let positional = positional.iter().cloned();
    let named = named.iter().map(|(var, val)| (key(var), val.clone()));
    let obj = [
        (key("positional"), positional.collect()),
        (key("named"), Val::obj(named.collect())),
    ];
    Val::obj(obj.into_iter().collect())
}

#[derive(Debug)]
enum Error {
    Io(Option<String>, io::Error),
    Report(Vec<FileReports<PathBuf>>),
    Parse(String),
    Jaq(jaq_all::json::Error),
    Halt(i32),
    FalseOrNull,
    NoOutput,
}

struct ErrorColor<'e>(&'e Error, Paint);

impl<'e> ErrorColor<'e> {
    fn new(e: &'e Error, color: bool) -> Self {
        let with_color = |f: &mut Formatter, c: &Option<Color>, d: &dyn Display| match c {
            Some(color) => color.ansi(f, d),
            None => d.fmt(f),
        };
        let without_color = |f: &mut Formatter, _c: &_, d: &dyn Display| d.fmt(f);
        Self(e, if color { with_color } else { without_color })
    }
}

impl fmt::Display for ErrorColor<'_> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        let Self(error, color) = self;
        match error {
            Error::FalseOrNull | Error::NoOutput | Error::Halt(_) => Ok(()),
            Error::Io(_, e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
            Error::Io(prefix, e) => {
                write!(f, "Error: ")?;
                if let Some(p) = prefix {
                    write!(f, "{p}: ")?;
                }
                writeln!(f, "{e}")
            }
            Error::Report(reports) => reports.iter().try_for_each(|fr| {
                FileReportsDisp::new(fr)
                    .with_paint(*color)
                    .with_path(|p| format!("[{}]", p.display()))
                    .fmt(f)
            }),
            Error::Parse(e) => writeln!(f, "Error: failed to parse: {e}"),
            Error::Jaq(e) => writeln!(f, "Error: {e}"),
        }
    }
}

impl Error {
    fn report(self) -> ExitCode {
        ExitCode::from(match self {
            Self::FalseOrNull => 1,
            Self::Io(_, e) if e.kind() == io::ErrorKind::BrokenPipe => 141,
            Self::Io(_, _) => 2,
            Self::Report(_) => 3,
            Self::NoOutput => 4,
            Self::Parse(_) | Self::Jaq(_) => 5,
            // ExitCode ~= u8, but exit_code: i32
            Self::Halt(exit_code) => exit_code,
        })
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(None, e)
    }
}
