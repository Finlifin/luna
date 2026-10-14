//! Source compilation, IR export and explicit bounded comptime invocation.

use luna::{cli, frontend};
mod output;

use cli::Command;
use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let command = match cli::parse(std::env::args_os().skip(1)) {
        Ok(command) => command,
        Err(message) => {
            report_error(&message);
            let _ = writeln!(io::stderr().lock(), "\n{}", cli::USAGE);
            return ExitCode::from(2);
        }
    };

    let result = match command {
        Command::Help => output::write_stdout(cli::HELP),
        Command::Invoke {
            options,
            entry,
            fuel,
        } => frontend::compile(&options)
            .and_then(|artifact| {
                let frontend::Artifact::Nir(mut package) = artifact else {
                    return Err("comptime invocation requires NIR".into());
                };
                // Optional program optimization follows frontend validation.
                // Unknown retains checked NIR; predicates still evaluate once.
                elaboration::verification::optimization::elide_proven_checks(
                    &mut package,
                    &mut elaboration::verification::Limits::default(),
                );
                luna::invocation::invoke_in_package(&package, 0, &entry, fuel).map(|value| value.to_string())
            })
            .and_then(|value| {
                if let Some(path) = &options.output {
                    output::write_file(&options.input, path, &value)
                } else {
                    output::write_stdout(&value)
                }
            }),
        Command::Interpret {
            options,
            inline_functions,
            entry,
            protocols,
            fuel,
        } => frontend::compile_with_resource_protocols(&options,
            protocols.as_ref().map(|(copy, _)| copy.as_str()),
            protocols.as_ref().map(|(_, drop)| drop.as_str()))
            .and_then(|artifact| {
                let frontend::Artifact::Nir(package) = artifact else {
                    return Err("reference execution requires NIR".into());
                };
                match protocols {
                    Some((copy, drop)) if inline_functions => luna::execution::interpret_inline_with_protocols_in_package(
                        &package, 0, &entry, &copy, &drop, fuel,
                    ),
                    Some((copy, drop)) => luna::execution::interpret_with_protocols_in_package(
                        &package, 0, &entry, &copy, &drop, fuel,
                    ),
                    None => luna::execution::interpret_in_package(&package, 0, &entry, fuel),
                }
                .map(|value| value.to_string())
            })
            .and_then(|value| {
                if let Some(path) = &options.output {
                    output::write_file(&options.input, path, &value)
                } else {
                    output::write_stdout(&value)
                }
            }),
        Command::Instantiate {
            options,
            entry,
            fuel,
        } => frontend::compile(&options)
            .and_then(|artifact| {
                let frontend::Artifact::Nir(package) = artifact else {
                    return Err("instantiation requires NIR".into());
                };
                luna::instantiation::instantiate_in_package(&package, 0, &entry, fuel)
                    .map(|package| package.to_string())
            })
            .and_then(|dump| {
                if let Some(path) = &options.output {
                    output::write_file(&options.input, path, &dump)
                } else {
                    output::write_stdout(&dump)
                }
            }),
        Command::AnalyzeResources {
            options,
            entry,
            copy_protocol,
            drop_protocol,
            fuel,
        } => frontend::compile_with_resource_protocols(&options, Some(&copy_protocol), drop_protocol.as_deref())
            .and_then(|artifact| {
                let frontend::Artifact::Nir(package) = artifact else {
                    return Err("resource analysis requires NIR".into());
                };
                luna::resources::analyze_with_protocols_in_package(
                    &package,
                    0,
                    &entry,
                    &copy_protocol,
                    drop_protocol.as_deref(),
                    fuel,
                )
                .map(|analysis| analysis.to_string())
            })
            .and_then(|dump| {
                if let Some(path) = &options.output {
                    output::write_file(&options.input, path, &dump)
                } else {
                    output::write_stdout(&dump)
                }
            }),
        Command::Compile(options) => frontend::compile(&options)
            .map(|artifact| artifact.to_string())
            .and_then(|dump| {
                if let Some(path) = &options.output {
                    output::write_file(&options.input, path, &dump)
                } else {
                    output::write_stdout(&dump)
                }
            }),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            report_error(&message);
            ExitCode::FAILURE
        }
    }
}

fn report_error(message: &str) {
    // Even an unavailable stderr must not turn a user-facing error into a panic.
    let _ = writeln!(io::stderr().lock(), "luna: error: {message}");
}
