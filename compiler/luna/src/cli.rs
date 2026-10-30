use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

mod subcommands;
pub use subcommands::{HELP, USAGE};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emit {
    Hir,
    Ast,
    Nir,
    Attributes,
    MergedAttributes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub dispatch_fuel: Option<usize>,
    pub comptime_intrinsics: Vec<(nir::intrinsics::Operation, String)>,
    pub build_modes: crate::build_context::ModeOptions,
    pub build_targets: crate::build_context::TargetOptions,
    pub list_pattern_protocol: Option<String>,
    pub iterator_protocol: Option<String>,
    pub into_iterator_protocol: Option<String>,
    pub index_protocol: Option<String>,
    pub ord_protocol: Option<String>,
    pub eq_protocol: Option<String>,
    pub literal_into_protocol: Option<String>,
    pub arithmetic_protocols: [Option<String>; 7],
    pub input: PathBuf,
    pub emit: Emit,
    pub output: Option<PathBuf>,
    pub sysroot: Option<PathBuf>,
    pub splice_protocol: Option<String>,
    pub meta_list_type: Option<String>,
    pub tuple_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOutput {
    Llvm,
    Assembly,
    Object,
    Bitcode,
    Executable,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    PackageStream {
        capacity: usize,
    },
    Help,
    HelpTopic(String),
    Check(Options),
    CheckPackages(Box<crate::package_cli::Request>),
    Packages(Box<crate::package_cli::Request>),
    Compile(Options),
    Host {
        host_fma: Option<String>,
        host_sqrt: Option<String>,
        host_copysign: Option<String>,
        host_intrinsics: Vec<(nir::intrinsics::Operation, String)>,
        options: Options,
        entry: String,
        fuel: u64,
        output_kind: HostOutput,
        discard_result: bool,
        inline_functions: bool,
        optimization: Option<crate::host::OptimizationLevel>,
        protocols: Option<(String, String)>,
    },
    AnalyzeTextRegions {
        options: Options,
        entry: String,
        fuel: u64,
        inline_functions: bool,
    },
    AnalyzeResources {
        options: Options,
        entry: String,
        copy_protocol: String,
        drop_protocol: Option<String>,
        fuel: u64,
    },
    Test {
        host_fma: Option<String>,
        host_sqrt: Option<String>,
        host_copysign: Option<String>,
        host_intrinsics: Vec<(nir::intrinsics::Operation, String)>,
        timeout: std::time::Duration,
        optimization: Option<crate::host::OptimizationLevel>,
        native: bool,
        list_only: bool,
        filter: Option<String>,
        options: Options,
        inline_functions: bool,
        protocols: Option<(String, String)>,
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
        definition_fuel: usize,
        protocols: Option<(String, String)>,
        options: Options,
        entry: String,
        fuel: u64,
    },
}

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    subcommands::parse(args.into_iter().collect())
}

pub(crate) fn parse_options(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let args: Vec<_> = args.into_iter().collect();
    if args
        .first()
        .is_some_and(|argument| argument == "--packages-stream-v1")
    {
        return match args.as_slice() {
            [_] => Ok(Command::PackageStream { capacity: 8 }),
            [_, option] if option == "--help" => Ok(Command::Help),
            [_, option, value] if option == "--cache-entries" => {
                let value = value
                    .to_str()
                    .ok_or("cache capacity must be an unsigned decimal integer")?;
                if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err("cache capacity must be an unsigned decimal integer".into());
                }
                Ok(Command::PackageStream {
                    capacity: value
                        .parse()
                        .map_err(|_| "cache capacity is out of range")?,
                })
            }
            _ => Err("usage: --packages-stream-v1 [--cache-entries <count>]".into()),
        };
    }

    if args
        .first()
        .is_some_and(|argument| argument == "--packages-v1")
    {
        if args.len() == 2 && args[1] == "--help" {
            return Ok(Command::Help);
        }
        return crate::package_cli::parse(args.into_iter().skip(1))
            .map(|request| Command::Packages(Box::new(request)));
    }
    if args.len() == 1 && args[0] == "--help" {
        return Ok(Command::Help);
    }
    let mut args = args.into_iter();
    let mut input = None;
    let mut emit = None;
    let mut output = None;
    let mut sysroot = None;
    let mut build_modes = crate::build_context::ModeOptions::default();
    let mut build_targets = crate::build_context::TargetOptions::default();
    let mut splice_protocol = None;
    let mut meta_list_type = None;
    let mut list_pattern_protocol = None;
    let mut iterator_protocol = None;
    let mut into_iterator_protocol = None;
    let mut index_protocol = None;
    let mut ord_protocol = None;
    let mut eq_protocol = None;
    let mut literal_into_protocol = None;
    let mut tuple_type = None;
    let mut arithmetic_protocols: [Option<String>; 7] = Default::default();
    let mut positional = false;
    let mut entry = None;
    let mut instantiate = None;
    let mut host = None;
    let mut host_fma = None;
    let mut host_sqrt = None;
    let mut host_copysign = None;
    let mut host_intrinsics = Vec::new();
    let mut comptime_intrinsics = Vec::new();
    let mut optimization = None;
    let mut interpret = None;
    let mut tests = false;
    let mut native_tests = false;
    let mut list_only = false;
    let mut test_filter = None;
    let mut test_timeout = None;
    let mut inline_functions = false;
    let mut discard_result = false;
    let mut resources = None;
    let mut text_regions = None;
    let mut copy_protocol = None;
    let mut drop_protocol = None;
    let mut fuel = None;
    let mut dispatch_fuel = None;
    let mut definition_fuel = None;
    while let Some(arg) = args.next() {
        if !positional {
            match arg.to_str() {
                Some("--") => {
                    positional = true;
                    continue;
                }
                Some("--help") => return Err("--help must be used alone".into()),
                Some(
                    flag @ ("--build-target"
                    | "--package-build-target"
                    | "--build-feature"
                    | "--package-build-feature"),
                ) => {
                    let owner = if flag.starts_with("--package-") {
                        let value = option_value(&mut args, flag)?
                            .into_string()
                            .map_err(|_| "build target identity must be UTF-8")?;
                        Some(crate::build_context::TargetOptions::package_identity(
                            &value,
                        )?)
                    } else {
                        None
                    };
                    let first = option_value(&mut args, flag)?
                        .into_string()
                        .map_err(|_| "build target input must be UTF-8")?;
                    if flag.ends_with("-feature") {
                        build_targets.add_feature(owner, &first)?;
                    } else {
                        let os = option_value(&mut args, flag)?
                            .into_string()
                            .map_err(|_| "build target OS must be UTF-8")?;
                        let bits = option_value(&mut args, flag)?
                            .into_string()
                            .map_err(|_| "build target width must be UTF-8")?;
                        build_targets.set_target(owner, first, os, &bits)?;
                    }
                    continue;
                }
                Some("--build-mode") => {
                    let value = option_value(&mut args, "--build-mode")?
                        .into_string()
                        .map_err(|_| "build mode must be UTF-8")?;
                    build_modes.set_primary(value)?;
                    continue;
                }
                Some("--package-build-mode") => {
                    let value = option_value(&mut args, "--package-build-mode")?
                        .into_string()
                        .map_err(|_| "package build mode must be UTF-8")?;
                    build_modes.add_package(&value)?;
                    continue;
                }
                Some(
                    flag @ ("--build" | "--emit-llvm" | "--emit-object" | "--emit-asm"
                    | "--emit-bitcode"),
                ) => {
                    if host.is_some() {
                        return Err("only one host action is allowed".into());
                    }
                    let name = option_value(&mut args, flag)?
                        .into_string()
                        .map_err(|_| "entry must be UTF-8")?;
                    host = Some((
                        name,
                        match flag {
                            "--emit-llvm" => HostOutput::Llvm,
                            "--emit-asm" => HostOutput::Assembly,
                            "--emit-object" => HostOutput::Object,
                            "--emit-bitcode" => HostOutput::Bitcode,
                            _ => HostOutput::Executable,
                        },
                    ));
                    continue;
                }
                Some("--host-fma") => {
                    if host_fma.is_some() {
                        return Err("--host-fma may only be specified once".into());
                    }
                    host_fma = Some(
                        option_value(&mut args, "--host-fma")?
                            .into_string()
                            .map_err(|_| "host FMA selector must be UTF-8")?,
                    );
                    continue;
                }
                Some("--host-sqrt") => {
                    if host_sqrt.is_some() {
                        return Err("--host-sqrt may only be specified once".into());
                    }
                    host_sqrt = Some(
                        option_value(&mut args, "--host-sqrt")?
                            .into_string()
                            .map_err(|_| "host sqrt selector must be UTF-8")?,
                    );
                    continue;
                }
                Some("--host-copysign") => {
                    if host_copysign.is_some() {
                        return Err("--host-copysign may only be specified once".into());
                    }
                    host_copysign = Some(
                        option_value(&mut args, "--host-copysign")?
                            .into_string()
                            .map_err(|_| "host copysign selector must be UTF-8")?,
                    );
                    continue;
                }
                Some(flag @ ("--host-intrinsic" | "--comptime-intrinsic")) => {
                    let value = option_value(&mut args, flag)?
                        .into_string()
                        .map_err(|_| "host intrinsic selector must be UTF-8")?;
                    let (operation, selector) = value
                        .split_once('=')
                        .ok_or("--host-intrinsic requires operation=package.Enum.variant")?;
                    let operation =
                        nir::intrinsics::Operation::from_name(operation).ok_or_else(|| {
                            let names = nir::intrinsics::Operation::ALL
                                .iter()
                                .map(|operation| operation.name())
                                .collect::<Vec<_>>()
                                .join(", ");
                            format!("unknown host intrinsic operation (expected one of: {names})")
                        })?;
                    if selector.is_empty() {
                        return Err("host intrinsic selector must not be empty".into());
                    }
                    if flag == "--comptime-intrinsic" {
                        comptime_intrinsics.push((operation, selector.to_owned()));
                    } else {
                        host_intrinsics.push((operation, selector.to_owned()));
                    }
                    continue;
                }
                Some("--opt-level") => {
                    if optimization.is_some() {
                        return Err("--opt-level may only be specified once".into());
                    }
                    use crate::host::OptimizationLevel::*;
                    optimization = Some(match option_value(&mut args, "--opt-level")?.to_str() {
                        Some("0") => O0,
                        Some("1") => O1,
                        Some("2") => O2,
                        Some("3") => O3,
                        _ => return Err("--opt-level requires 0, 1, 2 or 3".into()),
                    });
                    continue;
                }
                Some("--emit") => {
                    if emit.is_some() {
                        return Err("--emit may only be specified once".into());
                    }
                    let value = option_value(&mut args, "--emit")?;
                    emit = Some(match value.to_str() {
                        Some("hir") => Emit::Hir,
                        Some("ast") => Emit::Ast,
                        Some("nir") => Emit::Nir,
                        Some("attributes") => Emit::Attributes,
                        Some("merged-attributes") => Emit::MergedAttributes,
                        _ => {
                            return Err(format!(
                                "invalid --emit value '{}': expected hir, ast, nir, attributes or merged-attributes",
                                value.to_string_lossy()
                            ));
                        }
                    });
                    continue;
                }
                Some("--analyze-text-regions") => {
                    if text_regions.is_some() {
                        return Err("--analyze-text-regions may only be specified once".into());
                    }
                    text_regions = Some(
                        option_value(&mut args, "--analyze-text-regions")?
                            .into_string()
                            .map_err(|_| "text region entry must be valid UTF-8")?,
                    );
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
                Some("--discard-result") => {
                    if std::mem::replace(&mut discard_result, true) {
                        return Err("--discard-result may only be specified once".into());
                    }
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
                Some("--test-filter") => {
                    if test_filter.is_some() {
                        return Err("--test-filter may only be specified once".into());
                    }
                    test_filter = Some(
                        args.next()
                            .filter(|value| !is_option(value))
                            .ok_or("--test-filter requires a value")?
                            .into_string()
                            .map_err(|_| "test filter must be UTF-8")?,
                    );
                    continue;
                }
                Some("--test-timeout-ms") => {
                    if test_timeout.is_some() {
                        return Err("--test-timeout-ms supplied more than once".into());
                    }
                    let value = option_value(&mut args, "--test-timeout-ms")?;
                    let text = value
                        .to_str()
                        .ok_or("--test-timeout-ms must be a positive integer")?;
                    let milliseconds =
                        if !text.is_empty() && text.bytes().all(|c| c.is_ascii_digit()) {
                            text.parse::<u64>().ok().filter(|value| *value > 0)
                        } else {
                            None
                        }
                        .ok_or("--test-timeout-ms must be a positive integer in milliseconds")?;
                    test_timeout = Some(std::time::Duration::from_millis(milliseconds));
                    continue;
                }
                Some("--test" | "--test-native" | "--list-tests") => {
                    if interpret.is_some() {
                        return Err(
                            "--test/--list-tests conflicts with --interpret or another test action"
                                .into(),
                        );
                    }
                    list_only = arg == "--list-tests";
                    native_tests = arg == "--test-native";
                    tests = true;
                    interpret = Some("$tests".to_owned());
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
                Some("--definition-fuel") => {
                    if definition_fuel.is_some() {
                        return Err("--definition-fuel may only be specified once".into());
                    }
                    definition_fuel = Some(
                        option_value(&mut args, "--definition-fuel")?
                            .to_str()
                            .and_then(|value| value.parse::<usize>().ok())
                            .filter(|steps| *steps > 0)
                            .ok_or("--definition-fuel requires a positive integer")?,
                    );
                    continue;
                }
                Some("--dispatch-fuel") => {
                    if dispatch_fuel.is_some() {
                        return Err("--dispatch-fuel may only be specified once".into());
                    }
                    dispatch_fuel = Some(
                        option_value(&mut args, "--dispatch-fuel")?
                            .to_str()
                            .and_then(|value| value.parse::<usize>().ok())
                            .filter(|steps| *steps > 0)
                            .ok_or("--dispatch-fuel requires a positive integer")?,
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
                Some("--list-pattern-protocol") => {
                    if list_pattern_protocol.is_some() {
                        return Err("--list-pattern-protocol may only be specified once".into());
                    }
                    list_pattern_protocol = Some(
                        option_value(&mut args, "--list-pattern-protocol")?
                            .into_string()
                            .map_err(|_| "ListPatternView protocol selector must be valid UTF-8")?,
                    );
                    continue;
                }
                Some("--iterator-protocol") => {
                    if iterator_protocol.is_some() {
                        return Err("--iterator-protocol may only be specified once".into());
                    }
                    iterator_protocol = Some(
                        option_value(&mut args, "--iterator-protocol")?
                            .into_string()
                            .map_err(|_| "Iterator protocol selector must be valid UTF-8")?,
                    );
                    continue;
                }
                Some("--into-iterator-protocol") => {
                    if into_iterator_protocol.is_some() {
                        return Err("--into-iterator-protocol may only be specified once".into());
                    }
                    into_iterator_protocol = Some(
                        option_value(&mut args, "--into-iterator-protocol")?
                            .into_string()
                            .map_err(|_| "Iterator protocol selector must be valid UTF-8")?,
                    );
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
                Some("--ord-protocol") => {
                    if ord_protocol.is_some() {
                        return Err("--ord-protocol may only be specified once".into());
                    }
                    ord_protocol = Some(
                        option_value(&mut args, "--ord-protocol")?
                            .into_string()
                            .map_err(|_| "Ord protocol selector must be valid UTF-8")?,
                    );
                    continue;
                }
                Some("--eq-protocol") => {
                    if eq_protocol.is_some() {
                        return Err("--eq-protocol may only be specified once".into());
                    }
                    eq_protocol = Some(
                        option_value(&mut args, "--eq-protocol")?
                            .into_string()
                            .map_err(|_| "Eq protocol selector must be valid UTF-8")?,
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
                Some(option @ ("--meta-list-type" | "--tuple-type")) => {
                    let target = if option == "--meta-list-type" {
                        &mut meta_list_type
                    } else {
                        &mut tuple_type
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
    if text_regions.is_some()
        && (host.is_some()
            || tests
            || resources.is_some()
            || interpret.is_some()
            || instantiate.is_some()
            || entry.is_some()
            || emit.is_some()
            || copy_protocol.is_some()
            || drop_protocol.is_some())
    {
        return Err("--analyze-text-regions cannot be combined with other actions or resource protocol flags".into());
    }
    if test_timeout.is_some() && !native_tests {
        return Err("--test-timeout-ms requires --test-native".into());
    }
    if !host_intrinsics.is_empty() && host.is_none() && !native_tests {
        return Err(
            "--host-intrinsic requires --build, --emit-object, --emit-bitcode, --emit-asm, --emit-llvm or --test-native".into(),
        );
    }
    if host_fma.is_some() && host.is_none() && !native_tests {
        return Err(
            "--host-fma requires --build, --emit-object, --emit-bitcode, --emit-asm, --emit-llvm or --test-native"
                .into(),
        );
    }
    if host_sqrt.is_some() && host.is_none() && !native_tests {
        return Err(
            "--host-sqrt requires --build, --emit-object, --emit-bitcode, --emit-asm, --emit-llvm or --test-native"
                .into(),
        );
    }
    if host_copysign.is_some() && host.is_none() && !native_tests {
        return Err(
            "--host-copysign requires --build, --emit-object, --emit-bitcode, --emit-asm, --emit-llvm or --test-native".into(),
        );
    }
    if optimization.is_some() && host.is_none() && !native_tests {
        return Err(
            "--opt-level requires --build, --emit-object, --emit-bitcode, --emit-asm, --emit-llvm or --test-native"
                .into(),
        );
    }
    if let Some((_, output_kind)) = &host {
        if tests
            || resources.is_some()
            || interpret.is_some()
            || instantiate.is_some()
            || entry.is_some()
            || emit.is_some()
        {
            return Err("host actions cannot be combined with other actions".into());
        }
        if matches!(
            output_kind,
            HostOutput::Object | HostOutput::Bitcode | HostOutput::Executable
        ) && output.is_none()
        {
            return Err(match output_kind {
                HostOutput::Object => "--emit-object requires --output <object>",
                HostOutput::Bitcode => "--emit-bitcode requires --output <bitcode>",
                _ => "--build requires --output <executable>",
            }
            .into());
        }
    }
    // A sysroot can supply the pair after argument parsing. Execution must
    // still reject missing identities; merely naming a sysroot grants nothing.
    let resource_pair_pending =
        sysroot.is_some() || (copy_protocol.is_some() && drop_protocol.is_some());
    if discard_result && (host.is_none() || !resource_pair_pending) {
        return Err("--discard-result requires --build, --emit-object, --emit-bitcode, --emit-asm or --emit-llvm with Copy/Drop bindings from --sysroot or --copy-protocol and --drop-protocol".into());
    }
    if inline_functions
        && text_regions.is_none()
        && ((interpret.is_none() && host.is_none() && !tests) || !resource_pair_pending)
    {
        return Err(
            "--inline-functions requires --analyze-text-regions, or --interpret, --build, --emit-object, --emit-bitcode, --emit-asm, --emit-llvm or tests with Copy/Drop bindings from --sysroot or --copy-protocol and --drop-protocol"
                .into(),
        );
    }
    if list_only && inline_functions {
        return Err("--list-tests cannot be combined with --inline-functions".into());
    }
    if test_filter.is_some() && !tests {
        return Err("--test-filter requires --test, --test-native or --list-tests".into());
    }
    if resources.is_some()
        && (interpret.is_some() || instantiate.is_some() || entry.is_some() || emit.is_some())
    {
        return Err("--analyze-resources cannot be combined with --interpret, --instantiate, --invoke-comptime or --emit".into());
    }
    if interpret.is_some() || entry.is_some() || host.is_some() {
        if copy_protocol.is_some() != drop_protocol.is_some() {
            return Err(
                "resource-checked invocation requires both --copy-protocol and --drop-protocol"
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
    if host.is_none()
        && text_regions.is_none()
        && resources.is_none()
        && entry.is_none()
        && instantiate.is_none()
        && interpret.is_none()
        && fuel.is_some()
    {
        return Err(
            "--fuel requires --invoke-comptime, --instantiate, --interpret or --analyze-resources or --analyze-text-regions"
                .into(),
        );
    }
    if definition_fuel.is_some() && entry.is_none() {
        return Err("--definition-fuel requires --invoke-comptime".into());
    }
    let options = Options {
        dispatch_fuel,
        comptime_intrinsics,
        build_modes,
        build_targets,
        input: input.ok_or_else(|| "an input path is required".to_string())?,
        emit: if host.is_some()
            || text_regions.is_some()
            || resources.is_some()
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
        arithmetic_protocols,
        list_pattern_protocol,
        iterator_protocol,
        into_iterator_protocol,
        index_protocol,
        ord_protocol,
        eq_protocol,
        literal_into_protocol,
    };
    options.build_modes.validate_stage(options.emit)?;
    options.build_targets.validate_stage(options.emit)?;
    crate::frontend::source_protocols(&options).validate_stage(options.emit)?;
    if let Some(entry) = text_regions {
        return Ok(Command::AnalyzeTextRegions {
            options,
            entry,
            fuel: fuel.unwrap_or(100_000),
            inline_functions,
        });
    }
    if let Some((entry, output_kind)) = host {
        return Ok(Command::Host {
            host_fma,
            host_sqrt,
            host_copysign,
            host_intrinsics,
            options,
            entry,
            output_kind,
            discard_result,
            inline_functions,
            optimization,
            protocols: copy_protocol.zip(drop_protocol),
            fuel: fuel.unwrap_or(100_000),
        });
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
    if tests {
        return Ok(Command::Test {
            host_fma,
            host_sqrt,
            host_copysign,
            host_intrinsics,
            timeout: test_timeout.unwrap_or(std::time::Duration::from_secs(10)),
            optimization,
            native: native_tests,
            list_only,
            filter: test_filter,
            options,
            inline_functions,
            protocols: copy_protocol.zip(drop_protocol),
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
            definition_fuel: definition_fuel.unwrap_or(1_000_000),
            protocols: copy_protocol.zip(drop_protocol),
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
                dispatch_fuel: None,
                comptime_intrinsics: Vec::new(),
                input: "source 文 件.fl".into(),
                emit: Emit::Hir,
                output: None,
                build_modes: Default::default(),
                build_targets: Default::default(),
                sysroot: None,
                splice_protocol: None,
                meta_list_type: None,
                index_protocol: None,
                ord_protocol: None,
                eq_protocol: None,
                list_pattern_protocol: None,
                iterator_protocol: None,
                into_iterator_protocol: None,
                literal_into_protocol: None,
                arithmetic_protocols: Default::default(),
                tuple_type: None,
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
                dispatch_fuel: None,
                comptime_intrinsics: Vec::new(),
                input: "source.fl".into(),
                emit: Emit::Ast,
                output: Some("out.lisp".into()),
                build_modes: Default::default(),
                build_targets: Default::default(),
                sysroot: Some("library".into()),
                splice_protocol: None,
                meta_list_type: None,
                index_protocol: None,
                ord_protocol: None,
                eq_protocol: None,
                list_pattern_protocol: None,
                iterator_protocol: None,
                into_iterator_protocol: None,
                literal_into_protocol: None,
                arithmetic_protocols: Default::default(),
                tuple_type: None,
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
            vec!["check"],
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
            definition_fuel: _,
            protocols: _,
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
