//! Public command grammar, independent of the versioned package wire format.
use super::{Command, OsString};

use super::help::help;

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
        if args.len() == 1 {
            return Ok(Command::Help);
        }
        let path = args[1..]
            .iter()
            .map(|part| part.to_str().ok_or("command must be UTF-8"))
            .collect::<Result<Vec<_>, _>>()?;
        return help(&path).map(Command::HelpTopic);
    }
    // Resolve only an exact command path here; option-bearing invocations
    // continue through normal parsing so invalid input is not silently ignored.
    if args
        .last()
        .is_some_and(|arg| arg == "--help" || arg == "-h")
    {
        let path = args[..args.len() - 1]
            .iter()
            .map(|part| part.to_str())
            .collect::<Option<Vec<_>>>();
        if let Some(path) = path {
            if let Ok(text) = help(&path) {
                return Ok(Command::HelpTopic(text));
            }
        }
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
        return Ok(Command::HelpTopic(help(&[first])?));
    }
    if first == "serve" {
        if args.get(1).is_none_or(|s| s != "packages-v1") {
            return Err("usage: luna serve packages-v1 [--cache-entries <count>]".into());
        }
        if args.len() == 3 && args[2] == "--help" {
            return Ok(Command::HelpTopic(help(&["serve"])?));
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
    if command == "bindings" {
        return Err("bindings is a help topic; use luna help bindings".into());
    }
    help(&[command])?;
    if matches!(command, "packages" | "serve") {
        return Err(format!("unsupported packages action '{command}'"));
    }
    if packages && args.len() == 3 && args[2] == "--help" {
        return Ok(Command::HelpTopic(help(&[command])?));
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
    let mut flags = std::collections::BTreeSet::new();
    let (mut input, mut entry, mut emit, mut engine) = (None, None, None, None);
    let mut list = false;
    let mut timeout = false;
    let mut index = start;
    while index < args.len() {
        let arg = &args[index];
        let flag = arg.to_str().unwrap_or("");
        if flag.starts_with('-') {
            flags.insert(if flag == "-o" { "--output" } else { flag });
        }
        match flag {
            "--help" | "-h" => {
                let mut path = Vec::new();
                if packages {
                    path.push("packages");
                }
                path.push(command);
                if let Some(topic) = analysis {
                    path.push(topic);
                }
                return Ok(Command::HelpTopic(help(&path)?));
            }
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
                    | "--comptime-list-pattern-protocol"
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
    // Validate public pipeline names before translating to internal options.
    // Diagnostics must teach the subcommand grammar, not legacy action flags.
    let host = command == "build"
        || (command == "dump" && matches!(format, Some("llvm" | "asm")))
        || (command == "test" && !list && engine.as_ref().is_none_or(|e| e == "native"));
    for flag in [
        "--opt-level",
        "--host-intrinsic",
        "--host-fma",
        "--host-sqrt",
        "--host-copysign",
    ] {
        if flags.contains(flag) && !host {
            return Err(format!(
                "{flag} requires build, test --engine native, or dump --emit llvm|asm"
            ));
        }
    }
    if command == "build" && !flags.contains("--output") {
        return Err("build requires -o <path> or --output <path>".into());
    }
    if flags.contains("--discard-result") && !(command == "build" || (command == "dump" && host)) {
        return Err("--discard-result requires build or dump --emit llvm|asm".into());
    }
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
