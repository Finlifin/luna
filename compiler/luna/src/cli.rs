use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

pub const USAGE: &str = "Usage: luna <input.fl> [--emit hir|ast|nir | --invoke-comptime <name> | --instantiate <name> | --interpret <name> [--inline-functions] [--copy-protocol <trait> --drop-protocol <trait>] | --analyze-resources <name> --copy-protocol <trait> [--drop-protocol <trait>]] [--fuel <steps>] [--output <path>] [--sysroot <library-path>] [--splice-protocol <trait|package::trait>] [--meta-list-type <package::definition> --tuple-type <package::definition>] [--optional-type <package::definition>] [--add-protocol <package::trait>] [--index-protocol <package::trait>] [--literal-into-protocol <package::trait>]\n       luna --help";

pub const HELP: &str = "Luna single-file frontend\n\nUsage: luna <input.fl> [--emit hir|ast|nir | --invoke-comptime <name> | --instantiate <name> | --interpret <name> [--inline-functions] [--copy-protocol <trait> --drop-protocol <trait>] | --analyze-resources <name> --copy-protocol <trait> [--drop-protocol <trait>]] [--fuel <steps>] [--output <path>] [--sysroot <library-path>] [--splice-protocol <trait|package::trait>] [--meta-list-type <package::definition> --tuple-type <package::definition>] [--optional-type <package::definition>] [--add-protocol <package::trait>] [--index-protocol <package::trait>] [--literal-into-protocol <package::trait>]\n       luna --help\n\n  --emit hir|ast|nir     Dump HIR (default), AST, or type-elaborated NIR\n  --invoke-comptime <name>  Invoke one unambiguous comptime function without supplied arguments\n  --instantiate <name>  Dump the concrete IIR dependency graph of a runtime entry\n  --interpret <name>  Reference-execute IIR; both protocol options enable checked cleanup\n  --inline-functions  Expand eligible inline fn calls (requires --interpret and both protocols)\n  --analyze-resources <name>  Report ownership obligations (does not lower Drop or certify full safety)\n  --copy-protocol <trait>  Explicit source Copy identity for analysis or checked execution\n  --drop-protocol <trait>  Source Drop identity; analyze plans or execute them with --interpret\n  --fuel <steps>    Positive per-stage/per-query work budget (default: 100000)\n  --output <path>    Write the dump or result to a file (default: stdout)\n  --sysroot <path>   Use a library directory containing builtin/ and std/\n                     NIR reads optional language-items.conf defaults; explicit bindings override them\n  --splice-protocol <trait|package::trait>  Bind runtime conversion of comptime constants (qualified module paths supported)\n  --meta-list-type  Bind List by package::definition or package::module::definition\n  --tuple-type      Bind Tuple by package::definition or package::module::definition (requires List)\n  --optional-type   Bind optional enum by package::definition or package::module::definition\n  --add-protocol / --sub-protocol / --mul-protocol / --div-protocol / --rem-protocol / --neg-protocol / --not-protocol\n                    Bind arithmetic by package::trait or package::module::trait\n  --index-protocol  Bind Index by package::trait or package::module::trait\n  --literal-into-protocol  Bind literal suffix conversion by package::trait or package::module::trait\n  --help            Show this help\n  --                Treat remaining arguments as input paths\n\nA main.fl/lib.fl input scans its parent as the source root; other inputs compile alone.\n\nExit status: 0 on success, 1 on compilation, invocation or I/O errors, 2 on invalid usage.\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emit {
    Hir,
    Ast,
    Nir,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub index_protocol: Option<String>,
    pub literal_into_protocol: Option<String>,
    pub arithmetic_protocols: [Option<String>; 7],
    pub input: PathBuf,
    pub emit: Emit,
    pub output: Option<PathBuf>,
    pub sysroot: Option<PathBuf>,
    pub splice_protocol: Option<String>,
    pub meta_list_type: Option<String>,
    pub tuple_type: Option<String>,
    pub optional_type: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    Compile(Options),
    AnalyzeResources {
        options: Options,
        entry: String,
        copy_protocol: String,
        drop_protocol: Option<String>,
        fuel: u64,
    },
    Interpret {
        options: Options,
        inline_functions: bool,
        entry: String,
        protocols: Option<(String, String)>,
        fuel: u64,
    },
    Instantiate {
        options: Options,
        entry: String,
        fuel: u64,
    },
    Invoke {
        options: Options,
        entry: String,
        fuel: u64,
    },
}

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let args: Vec<_> = args.into_iter().collect();
    if args.len() == 1 && args[0] == "--help" {
        return Ok(Command::Help);
    }
    let mut args = args.into_iter();
    let mut input = None;
    let mut emit = None;
    let mut output = None;
    let mut sysroot = None;
    let mut splice_protocol = None;
    let mut meta_list_type = None;
    let mut index_protocol = None;
    let mut literal_into_protocol = None;
    let mut tuple_type = None;
    let mut optional_type = None;
    let mut arithmetic_protocols: [Option<String>; 7] = Default::default();
    let mut positional = false;
    let mut entry = None;
    let mut instantiate = None;
    let mut interpret = None;
    let mut inline_functions = false;
    let mut resources = None;
    let mut copy_protocol = None;
    let mut drop_protocol = None;
    let mut fuel = None;
    while let Some(arg) = args.next() {
        if !positional {
            match arg.to_str() {
                Some("--") => {
                    positional = true;
                    continue;
                }
                Some("--help") => return Err("--help must be used alone".into()),
                Some("--emit") => {
                    if emit.is_some() {
                        return Err("--emit may only be specified once".into());
                    }
                    let value = option_value(&mut args, "--emit")?;
                    emit = Some(match value.to_str() {
                        Some("hir") => Emit::Hir,
                        Some("ast") => Emit::Ast,
                        Some("nir") => Emit::Nir,
                        _ => {
                            return Err(format!(
                                "invalid --emit value '{}': expected hir, ast or nir",
                                value.to_string_lossy()
                            ));
                        }
                    });
                    continue;
                }
                Some("--analyze-resources") => {
                    if resources.is_some() {
                        return Err("--analyze-resources may only be specified once".into());
                    }
                    resources = Some(
                        option_value(&mut args, "--analyze-resources")?
                            .into_string()
                            .map_err(|_| "resource entry name must be valid UTF-8")?,
                    );
                    continue;
                }
                Some("--drop-protocol") => {
                    if drop_protocol.is_some() {
                        return Err("--drop-protocol may only be specified once".into());
                    }
                    drop_protocol = Some(
                        option_value(&mut args, "--drop-protocol")?
                            .into_string()
                            .map_err(|_| "Drop protocol name must be valid UTF-8")?,
                    );
                    continue;
                }
                Some("--copy-protocol") => {
                    if copy_protocol.is_some() {
                        return Err("--copy-protocol may only be specified once".into());
                    }
                    copy_protocol = Some(
                        option_value(&mut args, "--copy-protocol")?
                            .into_string()
                            .map_err(|_| "Copy protocol name must be valid UTF-8")?,
                    );
                    continue;
                }
                Some("--inline-functions") => {
                    if inline_functions {
                        return Err("--inline-functions may only be specified once".into());
                    }
                    inline_functions = true;
                    continue;
                }
                Some("--interpret") => {
                    if interpret.is_some() {
                        return Err("--interpret may only be specified once".into());
                    }
                    interpret = Some(
                        option_value(&mut args, "--interpret")?
                            .into_string()
                            .map_err(|_| "entry name must be valid UTF-8")?,
                    );
                    continue;
                }
                Some("--instantiate") => {
                    if instantiate.is_some() {
                        return Err("--instantiate may only be specified once".into());
                    }
                    instantiate = Some(
                        option_value(&mut args, "--instantiate")?
                            .into_string()
                            .map_err(|_| "instance name must be valid UTF-8")?,
                    );
                    continue;
                }
                Some("--invoke-comptime") => {
                    if entry.is_some() {
                        return Err("--invoke-comptime may only be specified once".into());
                    }
                    entry = Some(
                        option_value(&mut args, "--invoke-comptime")?
                            .into_string()
                            .map_err(|_| "invocation name must be valid UTF-8")?,
                    );
                    continue;
                }
                Some("--fuel") => {
                    if fuel.is_some() {
                        return Err("--fuel may only be specified once".into());
                    }
                    let value = option_value(&mut args, "--fuel")?;
                    let steps = value
                        .to_str()
                        .and_then(|value| value.parse::<u64>().ok())
                        .filter(|steps| *steps > 0)
                        .ok_or("--fuel requires a positive integer")?;
                    fuel = Some(steps);
                    continue;
                }
                Some("--output") => {
                    if output.is_some() {
                        return Err("--output may only be specified once".into());
                    }
                    output = Some(PathBuf::from(option_value(&mut args, "--output")?));
                    continue;
                }
                Some("--index-protocol") => {
                    if index_protocol.is_some() {
                        return Err("--index-protocol may only be specified once".into());
                    }
                    index_protocol = Some(
                        option_value(&mut args, "--index-protocol")?
                            .into_string()
                            .map_err(|_| "Index protocol selector must be valid UTF-8")?,
                    );
                    continue;
                }
                Some("--literal-into-protocol") => {
                    if literal_into_protocol.is_some() {
                        return Err("--literal-into-protocol may only be specified once".into());
                    }
                    literal_into_protocol = Some(
                        option_value(&mut args, "--literal-into-protocol")?
                            .into_string()
                            .map_err(|_| "LiteralInto protocol selector must be valid UTF-8")?,
                    );
                    continue;
                }
                Some("--splice-protocol") => {
                    if splice_protocol.is_some() {
                        return Err("--splice-protocol may only be specified once".into());
                    }
                    splice_protocol = Some(
                        option_value(&mut args, "--splice-protocol")?
                            .into_string()
                            .map_err(|_| "Splice protocol name must be valid UTF-8")?,
                    );
                    continue;
                }
                Some(
                    option @ ("--add-protocol" | "--sub-protocol" | "--mul-protocol"
                    | "--div-protocol" | "--rem-protocol" | "--neg-protocol"
                    | "--not-protocol"),
                ) => {
                    let index = match option {
                        "--add-protocol" => 0,
                        "--sub-protocol" => 1,
                        "--mul-protocol" => 2,
                        "--div-protocol" => 3,
                        "--rem-protocol" => 4,
                        "--neg-protocol" => 5,
                        _ => 6,
                    };
                    if arithmetic_protocols[index].is_some() {
                        return Err(format!("{option} may only be specified once"));
                    }
                    let value = args
                        .next()
                        .ok_or_else(|| format!("{option} requires a source trait selector"))?
                        .into_string()
                        .map_err(|_| format!("{option} requires UTF-8"))?;
                    if value.starts_with('-') || value.is_empty() {
                        return Err(format!("{option} requires a source trait selector"));
                    }
                    arithmetic_protocols[index] = Some(value);
                    continue;
                }
                Some(option @ ("--meta-list-type" | "--tuple-type" | "--optional-type")) => {
                    let target = if option == "--meta-list-type" {
                        &mut meta_list_type
                    } else if option == "--tuple-type" {
                        &mut tuple_type
                    } else {
                        &mut optional_type
                    };
                    if target.is_some() {
                        return Err(format!("{option} may only be specified once"));
                    }
                    *target = Some(
                        option_value(&mut args, option)?
                            .into_string()
                            .map_err(|_| "type constructor selector must be valid UTF-8")?,
                    );
                    continue;
                }
                Some("--sysroot") => {
                    if sysroot.is_some() {
                        return Err("--sysroot may only be specified once".into());
                    }
                    sysroot = Some(PathBuf::from(option_value(&mut args, "--sysroot")?));
                    continue;
                }
                _ if is_option(&arg) => {
                    return Err(format!("unknown option '{}'", arg.to_string_lossy()));
                }
                _ => {}
            }
        }
        if arg.is_empty() {
            return Err("input path must not be empty".into());
        }
        if input.replace(PathBuf::from(arg)).is_some() {
            return Err("exactly one input path is required".into());
        }
    }
    if inline_functions
        && (interpret.is_none() || copy_protocol.is_none() || drop_protocol.is_none())
    {
        return Err(
            "--inline-functions requires --interpret with --copy-protocol and --drop-protocol"
                .into(),
        );
    }
    if resources.is_some()
        && (interpret.is_some() || instantiate.is_some() || entry.is_some() || emit.is_some())
    {
        return Err("--analyze-resources cannot be combined with --interpret, --instantiate, --invoke-comptime or --emit".into());
    }
    if interpret.is_some() {
        if copy_protocol.is_some() != drop_protocol.is_some() {
            return Err(
                "resource-checked --interpret requires both --copy-protocol and --drop-protocol"
                    .into(),
            );
        }
    } else {
        if drop_protocol.is_some() && resources.is_none() {
            return Err("--drop-protocol requires --analyze-resources or --interpret".into());
        }
        if resources.is_some() != copy_protocol.is_some() {
            return Err("--analyze-resources and --copy-protocol must be supplied together".into());
        }
    }
    if interpret.is_some() && (instantiate.is_some() || entry.is_some() || emit.is_some()) {
        return Err(
            "--interpret cannot be combined with --instantiate, --invoke-comptime or --emit".into(),
        );
    }
    if instantiate.is_some() && (entry.is_some() || emit.is_some()) {
        return Err("--instantiate cannot be combined with --invoke-comptime or --emit".into());
    }
    if entry.is_some() && emit.is_some() {
        return Err("--invoke-comptime cannot be combined with --emit".into());
    }
    if resources.is_none()
        && entry.is_none()
        && instantiate.is_none()
        && interpret.is_none()
        && fuel.is_some()
    {
        return Err(
            "--fuel requires --invoke-comptime, --instantiate, --interpret or --analyze-resources"
                .into(),
        );
    }
    let options = Options {
        input: input.ok_or_else(|| "an input path is required".to_string())?,
        emit: if resources.is_some()
            || entry.is_some()
            || instantiate.is_some()
            || interpret.is_some()
        {
            Emit::Nir
        } else {
            emit.unwrap_or(Emit::Hir)
        },
        output,
        sysroot,
        splice_protocol,
        meta_list_type,
        tuple_type,
        optional_type,
        arithmetic_protocols,
        index_protocol,
        literal_into_protocol,
    };
    if options.literal_into_protocol.is_some() && options.emit != Emit::Nir {
        return Err(
            "--literal-into-protocol requires --emit nir or an execution/analysis command".into(),
        );
    }
    if options.index_protocol.is_some() && options.emit != Emit::Nir {
        return Err("--index-protocol requires --emit nir or an execution/analysis command".into());
    }
    if options.splice_protocol.is_some() && options.emit != Emit::Nir {
        return Err(
            "--splice-protocol requires --emit nir or an execution/analysis command".into(),
        );
    }
    if (options.meta_list_type.is_some()
        || options.tuple_type.is_some()
        || options.optional_type.is_some())
        && options.emit != Emit::Nir
    {
        return Err(
            "type constructor bindings require --emit nir or an execution/analysis command".into(),
        );
    }
    if let Some(entry) = resources {
        return Ok(Command::AnalyzeResources {
            options,
            entry,
            copy_protocol: copy_protocol.expect("validated protocol option"),
            drop_protocol,
            fuel: fuel.unwrap_or(100_000),
        });
    }
    if let Some(entry) = interpret {
        return Ok(Command::Interpret {
            options,
            inline_functions,
            entry,
            protocols: copy_protocol.zip(drop_protocol),
            fuel: fuel.unwrap_or(100_000),
        });
    }
    if let Some(entry) = instantiate {
        return Ok(Command::Instantiate {
            options,
            entry,
            fuel: fuel.unwrap_or(100_000),
        });
    }
    Ok(match entry {
        Some(entry) => Command::Invoke {
            options,
            entry,
            fuel: fuel.unwrap_or(100_000),
        },
        None => Command::Compile(options),
    })
}

fn option_value(
    args: &mut impl Iterator<Item = OsString>,
    option: &str,
) -> Result<OsString, String> {
    args.next()
        .filter(|value| !value.is_empty() && !is_option(value))
        .ok_or_else(|| format!("{option} requires a value"))
}

fn is_option(value: &OsStr) -> bool {
    value.to_string_lossy().starts_with('-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(args: &[&str]) -> Result<Command, String> {
        parse(args.iter().map(OsString::from))
    }

    #[test]
    fn defaults_to_hir_and_stdout() {
        assert_eq!(
            arguments(&["source 文 件.fl"]).unwrap(),
            Command::Compile(Options {
                input: "source 文 件.fl".into(),
                emit: Emit::Hir,
                output: None,
                sysroot: None,
                splice_protocol: None,
                meta_list_type: None,
                index_protocol: None,
                literal_into_protocol: None,
                arithmetic_protocols: Default::default(),
                tuple_type: None,
                optional_type: None,
            })
        );
    }

    #[test]
    fn accepts_explicit_options_before_or_after_input() {
        assert_eq!(
            arguments(&[
                "--emit",
                "ast",
                "source.fl",
                "--sysroot",
                "library",
                "--output",
                "out.lisp"
            ])
            .unwrap(),
            Command::Compile(Options {
                input: "source.fl".into(),
                emit: Emit::Ast,
                output: Some("out.lisp".into()),
                sysroot: Some("library".into()),
                splice_protocol: None,
                meta_list_type: None,
                index_protocol: None,
                literal_into_protocol: None,
                arithmetic_protocols: Default::default(),
                tuple_type: None,
                optional_type: None,
            })
        );
    }

    #[test]
    fn help_is_a_standalone_command() {
        assert_eq!(arguments(&["--help"]).unwrap(), Command::Help);
        assert!(arguments(&["--help", "source.fl"]).is_err());
        assert!(arguments(&["--help", "--unknown"]).is_err());
    }

    #[test]
    fn rejects_missing_duplicate_and_unknown_arguments() {
        for args in [
            vec![],
            vec![""],
            vec!["a.fl", "b.fl"],
            vec!["a.fl", "--unknown"],
            vec!["a.fl", "--emit", "llvm"],
            vec!["a.fl", "--emit", "hir", "--emit", "ast"],
            vec!["a.fl", "--output", "a", "--output", "b"],
            vec!["a.fl", "--sysroot", "a", "--sysroot", "b"],
        ] {
            assert!(arguments(&args).is_err(), "accepted invalid args: {args:?}");
        }
        for option in ["--emit", "--output", "--sysroot"] {
            assert!(arguments(&["a.fl", option]).is_err());
            assert!(arguments(&["a.fl", option, ""]).is_err());
            assert!(arguments(&["a.fl", option, "--help"]).is_err());
        }
    }

    #[test]
    fn delimiter_allows_input_names_starting_with_dash() {
        let Command::Compile(options) = arguments(&["--", "--source.fl"]).unwrap() else {
            panic!("expected compile command");
        };
        assert_eq!(options.input, PathBuf::from("--source.fl"));
    }

    #[cfg(unix)]
    #[test]
    fn preserves_non_utf8_input_paths() {
        use std::os::unix::ffi::OsStringExt;
        let input = OsString::from_vec(b"source-\xff.fl".to_vec());
        let Command::Compile(options) = parse([input.clone()]).unwrap() else {
            panic!("expected compile command");
        };
        assert_eq!(options.input.as_os_str(), input.as_os_str());
    }
}

#[cfg(test)]
mod invocation_tests {
    use super::*;
    #[test]
    fn invocation_selects_nir_and_validates_its_own_options() {
        let Command::Invoke {
            options,
            entry,
            fuel,
        } = parse(
            [
                "--invoke-comptime",
                "BuildType",
                "input.fl",
                "--fuel",
                "123",
            ]
            .map(OsString::from),
        )
        .unwrap()
        else {
            panic!("expected invocation")
        };
        assert_eq!(options.emit, Emit::Nir);
        assert_eq!(entry, "BuildType");
        assert_eq!(fuel, 123);
        for args in [
            vec!["a.fl", "--fuel", "20"],
            vec!["a.fl", "--invoke-comptime"],
            vec!["a.fl", "--invoke-comptime", "run", "--emit", "nir"],
            vec![
                "a.fl",
                "--invoke-comptime",
                "run",
                "--invoke-comptime",
                "run",
            ],
            vec!["a.fl", "--invoke-comptime", "run", "--fuel", "0"],
            vec!["a.fl", "--invoke-comptime", "run", "--fuel", "lots"],
            vec![
                "a.fl",
                "--invoke-comptime",
                "run",
                "--fuel",
                "18446744073709551616",
            ],
            vec![
                "a.fl",
                "--invoke-comptime",
                "run",
                "--fuel",
                "1",
                "--fuel",
                "2",
            ],
        ] {
            assert!(parse(args.into_iter().map(OsString::from)).is_err());
        }
    }
}

#[cfg(test)]
mod instantiation_tests {
    use super::*;
    #[test]
    fn explicit_instantiation_selects_nir_and_has_its_own_budget() {
        let Command::Instantiate {
            options,
            entry,
            fuel,
        } = parse(["input.fl", "--instantiate", "entry", "--fuel", "37"].map(OsString::from))
            .unwrap()
        else {
            panic!("wrong command")
        };
        assert_eq!(options.emit, Emit::Nir);
        assert_eq!(entry, "entry");
        assert_eq!(fuel, 37);
        for arguments in [
            vec!["input.fl", "--instantiate", "entry", "--emit", "nir"],
            vec![
                "input.fl",
                "--instantiate",
                "entry",
                "--invoke-comptime",
                "run",
            ],
            vec![
                "input.fl",
                "--instantiate",
                "entry",
                "--instantiate",
                "other",
            ],
        ] {
            assert!(parse(arguments.into_iter().map(OsString::from)).is_err());
        }
    }
}
