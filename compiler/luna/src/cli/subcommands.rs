//! Public command grammar, independent of the versioned package wire format.
use super::{Command, OsString};

pub const USAGE: &str = "Usage: luna <command> [options]\n       luna help [command]";
pub const HELP: &str = "Luna — Flurry compiler\n\nUsage: luna <command> [options]\n\nCommands:\n  check       Type-check a source without emitting a dump\n  build       Build a host executable (or --emit object|bitcode)\n  test        Run tests (--engine native|reference; default native)\n  dump        Emit AST, HIR, NIR, IIR, LLVM, assembly or attributes\n  comptime    Invoke a comptime function\n  interpret   Execute a runtime entry in the reference interpreter\n  analyze     Inspect resources or text regions\n  packages    Apply a command to explicit package inputs\n  serve       Serve the versioned package stream protocol\n  help        Show command-specific help\n\nUse luna <command> --help for options and examples.\nSource-first invocations and --packages-v1/--packages-stream-v1 remain\ncompatibility entries; new automation should use subcommands.\n";
const COMMON: &str = "\nCommon options:\n  --sysroot <path>           Library directory\n  -o, --output <path>        Output file (otherwise stdout where supported)\n  --fuel <steps>             Positive action budget\n  --dispatch-fuel <steps>    Frontend preparation budget\n  --build-mode <mode>        Primary package mode\n  --build-target <arch> <os> <bits>\n  --build-feature <name=true|false>\n  --package-build-mode <identity=mode>\n  --package-build-target <identity> <arch> <os> <bits>\n  --package-build-feature <identity> <name=true|false>\n  --copy-protocol <trait>, --drop-protocol <trait>\n  --splice-protocol, --meta-list-type, --tuple-type, --index-protocol,\n  --ord-protocol, --eq-protocol, --list-pattern-protocol,\n  --literal-into-protocol, --iterator-protocol, --into-iterator-protocol,\n  --add-protocol, --sub-protocol, --mul-protocol, --div-protocol,\n  --rem-protocol, --neg-protocol, --not-protocol <source-selector>\n  --comptime-intrinsic <operation=selector> (repeatable)\nHost options: --opt-level 0|1|2|3, --inline-functions, --discard-result,\n  --host-intrinsic <operation=selector> (repeatable),\n  --host-fma <selector>, --host-sqrt <selector>, --host-copysign <selector>\nOptions are validated against the selected pipeline. Targets describe build\ncontext, not cross-compiling LLVM backend selection. Use -- before an input\npath beginning with a dash. Source and output paths need not be UTF-8.\n";

pub fn help(name: &str) -> Result<String, String> {
    let usage = match name {
        "check" => {
            "check <input.fl> [options]\n  Check through typed NIR; no artifact is printed. --output is not accepted."
        }
        "build" => {
            "build <input.fl> -o <path> [--entry <name>] [--emit executable|object|bitcode]\n  Entry defaults to main; output is required."
        }
        "test" => {
            "test <input.fl> [--engine native|reference] [--list] [--filter <text>]\n  --timeout-ms <ms> sets the native per-test deadline. --list only discovers\n  tests and cannot be combined with --engine or --timeout-ms."
        }
        "dump" => {
            "dump <input.fl> [--emit ast|hir|nir|iir|llvm|asm|attributes|merged-attributes]\n  Defaults to nir. --entry <name> (default main) applies to iir, llvm and asm."
        }
        "comptime" => {
            "comptime <input.fl> --entry <name> [--definition-fuel <steps>]\n  Invoke a comptime function and print its result."
        }
        "interpret" => {
            "interpret <input.fl> [--entry <name>]\n  Reference-execute a runtime entry (default main) and print its value."
        }
        "analyze" => {
            "analyze <resources|text-regions> <input.fl> [--entry <name>]\n  Entry defaults to main. resources requires --copy-protocol; optional\n  --drop-protocol supplies cleanup planning. Analysis is not verification."
        }
        "packages" => {
            "packages <check|build|test|dump|comptime|interpret|analyze> [command options]\n  --package <id> <identity> <name> <entry> (repeatable)\n  --primary <id> [--dependency <requester> <alias> <provider>]\n  [--builtin <id>] [--builtin-impl-package <id>] [--with-inputs]\n  [--package-metadata-v1 <hex>]\n  No positional source path; actions and options match source commands."
        }
        "serve" => {
            "serve packages-v1 [--cache-entries <count>]\n  Read versioned package requests from stdin; write protocol responses to\n  stdout. The request wire format remains unchanged. Default cache size: 8."
        }
        _ => return Err(format!("unknown command '{name}'")),
    };
    let options = if name == "serve" { "" } else { COMMON };
    Ok(format!("Usage: luna {usage}\n{options}"))
}
fn value(args: &[OsString], index: &mut usize, flag: &str) -> Result<OsString, String> {
    *index += 1;
    let value = args
        .get(*index)
        .ok_or_else(|| format!("{flag} requires a value"))?;
    if value.is_empty() || value.to_string_lossy().starts_with('-') {
        return Err(format!("{flag} requires a value"));
    }
    Ok(value.clone())
}
fn once(slot: &mut Option<OsString>, value: OsString, flag: &str) -> Result<(), String> {
    if slot.replace(value).is_some() {
        return Err(format!("{flag} may only be specified once"));
    }
    Ok(())
}

pub fn parse(args: Vec<OsString>) -> Result<Command, String> {
    let first = args.first().and_then(|s| s.to_str()).unwrap_or("");
    if args == [OsString::from("--version")] || args == [OsString::from("-V")] {
        return Ok(Command::HelpTopic(format!(
            "luna {}\n",
            env!("CARGO_PKG_VERSION")
        )));
    }
    if args.is_empty() || args == [OsString::from("--help")] || args == [OsString::from("-h")] {
        return Ok(Command::Help);
    }
    if first == "help" {
        return match args.as_slice() {
            [_] => Ok(Command::Help),
            [_, name] => Ok(Command::HelpTopic(help(
                name.to_str().ok_or("command must be UTF-8")?,
            )?)),
            _ => Err("usage: luna help [command]".into()),
        };
    }
    // Preserve the established source and v1 wire entrypoints. Unknown command
    // spellings are errors, not filenames silently sent to the compiler.
    if first.starts_with('-')
        || first.ends_with(".fl")
        || first.contains('/')
        || first.contains('\\')
        || args[0].to_str().is_none()
    {
        return super::parse_options(args);
    }
    if args.len() == 2 && (args[1] == "--help" || args[1] == "-h") {
        return Ok(Command::HelpTopic(help(first)?));
    }
    if first == "serve" {
        if args.get(1).is_none_or(|s| s != "packages-v1") {
            return Err("usage: luna serve packages-v1 [--cache-entries <count>]".into());
        }
        if args.len() == 3 && args[2] == "--help" {
            return Ok(Command::HelpTopic(help("serve")?));
        }
        return super::parse_options(
            std::iter::once(OsString::from("--packages-stream-v1")).chain(args.into_iter().skip(2)),
        );
    }
    let packages = first == "packages";
    let mut start = if packages { 2 } else { 1 };
    let command = if packages {
        args.get(1)
            .and_then(|s| s.to_str())
            .ok_or("packages requires an action")?
    } else {
        first
    };
    help(command)?;
    if matches!(command, "packages" | "serve") {
        return Err(format!("unsupported packages action '{command}'"));
    }
    if packages && args.len() == 3 && args[2] == "--help" {
        return Ok(Command::HelpTopic(help(command)?));
    }
    let analysis = if command == "analyze" {
        let topic = args
            .get(start)
            .and_then(|s| s.to_str())
            .ok_or("analyze requires resources or text-regions")?;
        if !matches!(topic, "resources" | "text-regions") {
            return Err("analyze requires resources or text-regions".into());
        }
        start += 1;
        Some(topic)
    } else {
        None
    };
    let mut forwarded = Vec::new();
    let (mut input, mut entry, mut emit, mut engine) = (None, None, None, None);
    let mut list = false;
    let mut timeout = false;
    let mut index = start;
    while index < args.len() {
        let arg = &args[index];
        let flag = arg.to_str().unwrap_or("");
        match flag {
            "--help" | "-h" => return Ok(Command::HelpTopic(help(command)?)),
            "--" => {
                if packages || input.is_some() || index + 2 != args.len() {
                    return Err("exactly one input path is required after --".into());
                }
                input = Some(args[index + 1].clone());
                break;
            }
            "--entry" => {
                if matches!(command, "check" | "test") {
                    return Err(format!("--entry is not valid for {command}"));
                }
                once(&mut entry, value(&args, &mut index, flag)?, flag)?;
            }
            "--emit" => {
                if !matches!(command, "build" | "dump") {
                    return Err(format!("--emit is not valid for {command}"));
                }
                once(&mut emit, value(&args, &mut index, flag)?, flag)?;
            }
            "--engine" => {
                if command != "test" {
                    return Err("--engine is only valid for test".into());
                }
                once(&mut engine, value(&args, &mut index, flag)?, flag)?;
            }
            "--list" => {
                if command != "test" || list {
                    return Err("--list is only valid once for test".into());
                }
                list = true;
            }
            "--filter" | "--timeout-ms" => {
                if command != "test" {
                    return Err(format!("{flag} is only valid for test"));
                }
                timeout |= flag == "--timeout-ms";
                forwarded.push(
                    if flag == "--filter" {
                        "--test-filter"
                    } else {
                        "--test-timeout-ms"
                    }
                    .into(),
                );
                forwarded.push(value(&args, &mut index, flag)?);
            }
            "-o" | "--output" => {
                if command == "check" {
                    return Err("check does not emit an output artifact".into());
                }
                forwarded.push("--output".into());
                forwarded.push(value(&args, &mut index, flag)?);
            }
            _ if flag.starts_with('-') => {
                let count = match flag {
                    "--inline-functions" | "--discard-result" => 0,
                    "--with-inputs" if packages && command != "check" => 0,
                    "--package" if packages => 4,
                    "--dependency" if packages => 3,
                    "--primary"
                    | "--builtin"
                    | "--builtin-impl-package"
                    | "--package-metadata-v1"
                        if packages =>
                    {
                        1
                    }
                    "--build-target" => 3,
                    "--package-build-target" => 4,
                    "--package-build-feature" => 2,
                    "--sysroot"
                    | "--fuel"
                    | "--dispatch-fuel"
                    | "--build-mode"
                    | "--build-feature"
                    | "--package-build-mode"
                    | "--copy-protocol"
                    | "--drop-protocol"
                    | "--splice-protocol"
                    | "--meta-list-type"
                    | "--tuple-type"
                    | "--index-protocol"
                    | "--ord-protocol"
                    | "--eq-protocol"
                    | "--list-pattern-protocol"
                    | "--literal-into-protocol"
                    | "--iterator-protocol"
                    | "--into-iterator-protocol"
                    | "--add-protocol"
                    | "--sub-protocol"
                    | "--mul-protocol"
                    | "--div-protocol"
                    | "--rem-protocol"
                    | "--neg-protocol"
                    | "--not-protocol"
                    | "--comptime-intrinsic"
                    | "--opt-level"
                    | "--host-intrinsic"
                    | "--host-fma"
                    | "--host-sqrt"
                    | "--host-copysign" => 1,
                    "--definition-fuel" if command == "comptime" => 1,
                    _ => return Err(format!("unknown option '{flag}' for {command}")),
                };
                forwarded.push(arg.clone());
                for _ in 0..count {
                    forwarded.push(value(&args, &mut index, flag)?);
                }
            }
            _ => {
                if packages {
                    return Err("packages takes --package inputs, not a source path".into());
                }
                once(&mut input, arg.clone(), "input path")?;
            }
        }
        index += 1;
    }
    let format = emit
        .as_ref()
        .map(|s| s.to_str().ok_or("format must be UTF-8"))
        .transpose()?;
    let supplied_entry = entry.is_some();
    let entry = entry.unwrap_or_else(|| "main".into());
    let mut action: Vec<OsString> = match command {
        "check" => vec!["--emit".into(), "nir".into()],
        "build" => vec![
            match format.unwrap_or("executable") {
                "executable" => "--build",
                "object" => "--emit-object",
                "bitcode" => "--emit-bitcode",
                _ => return Err("build --emit expects executable, object or bitcode".into()),
            }
            .into(),
            entry,
        ],
        "dump" => {
            let format = format.unwrap_or("nir");
            match format {
                "iir" | "llvm" | "asm" => vec![
                    match format {
                        "iir" => "--instantiate",
                        "llvm" => "--emit-llvm",
                        _ => "--emit-asm",
                    }
                    .into(),
                    entry,
                ],
                "ast" | "hir" | "nir" | "attributes" | "merged-attributes" => {
                    if supplied_entry {
                        return Err("--entry only applies to iir, llvm or asm dumps".into());
                    }
                    vec!["--emit".into(), format.into()]
                }
                _ => return Err("unknown dump format".into()),
            }
        }
        "comptime" => {
            if !supplied_entry {
                return Err("comptime requires --entry <name>".into());
            }
            vec!["--invoke-comptime".into(), entry]
        }
        "interpret" => vec!["--interpret".into(), entry],
        "analyze" => vec![
            if analysis == Some("resources") {
                "--analyze-resources"
            } else {
                "--analyze-text-regions"
            }
            .into(),
            entry,
        ],
        "test" => {
            if list && (engine.is_some() || timeout) {
                return Err("--list cannot be combined with --engine or --timeout-ms".into());
            }
            let engine = engine
                .as_ref()
                .map(|s| s.to_str().ok_or("engine must be UTF-8"))
                .transpose()?
                .unwrap_or("native");
            let flag = match engine {
                "native" => "--test-native",
                "reference" => "--test",
                _ => return Err("test --engine expects native or reference".into()),
            };
            vec![if list { "--list-tests" } else { flag }.into()]
        }
        _ => unreachable!(),
    };
    action.extend(forwarded);
    let result = if packages {
        crate::package_cli::parse(action).map(|request| Command::Packages(Box::new(request)))?
    } else {
        action.push("--".into());
        action.push(input.ok_or("source command requires an input path")?);
        super::parse_options(action)?
    };
    if command == "check" {
        match result {
            Command::Compile(options) => Ok(Command::Check(options)),
            Command::Packages(request) => Ok(Command::CheckPackages(request)),
            _ => unreachable!(),
        }
    } else {
        Ok(result)
    }
}
