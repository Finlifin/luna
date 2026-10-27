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
        Command::PackageStream { capacity } => {
            luna::package_stream::serve(&mut io::stdin().lock(), &mut io::stdout().lock(), capacity)
        }
        Command::Host {
            host_fma,
            host_sqrt,
            host_copysign,
            host_intrinsics,
            options,
            entry,
            fuel,
            output_kind,
            discard_result,
            inline_functions,
            optimization,
            protocols,
        } => frontend::compile_bound_for_native_host(
            &options,
            protocols.as_ref().map(|(copy, _)| copy.as_str()),
            protocols.as_ref().map(|(_, drop)| drop.as_str()),
            host_fma.as_deref(),
            host_sqrt.as_deref(),
            host_copysign.as_deref(),
            &host_intrinsics,
        )
        .and_then(|(artifact, bindings)| {
            let frontend::Artifact::Nir(package) = artifact else {
                return Err("host pipeline requires NIR".into());
            };
            let ir = if discard_result {
                luna::host::llvm_ir_discarding_result(
                    &package,
                    None,
                    &entry,
                    fuel,
                    (
                        bindings.copy.ok_or("host Copy binding is missing")?,
                        bindings.drop.ok_or("host Drop binding is missing")?,
                    ),
                    inline_functions,
                )?
            } else if inline_functions
                || protocols.is_some()
                || bindings.copy.is_some()
                || bindings.drop.is_some()
            {
                let lower = if inline_functions {
                    luna::host::llvm_ir_inlined_with_protocols
                } else {
                    luna::host::llvm_ir_with_protocols
                };
                lower(
                    &package,
                    &entry,
                    fuel,
                    bindings.copy.ok_or("host Copy binding is missing")?,
                    bindings.drop.ok_or("host Drop binding is missing")?,
                )?
            } else {
                luna::host::llvm_ir(&package, &entry, fuel)?
            };
            let toolchain = luna::host::Toolchain {
                optimization: optimization.unwrap_or_default(),
                ..Default::default()
            };
            if matches!(
                output_kind,
                cli::HostOutput::Llvm | cli::HostOutput::Assembly
            ) {
                let ir = if output_kind == cli::HostOutput::Assembly {
                    toolchain.assembly(&ir)?
                } else if optimization.is_some() {
                    toolchain.optimized_ir(&ir)?
                } else {
                    ir
                };
                if let Some(path) = &options.output {
                    output::write_file(&options.input, path, &ir)
                } else {
                    output::write_stdout(&ir)
                }
            } else {
                let kind = match output_kind {
                    cli::HostOutput::Object => luna::host::BinaryOutput::Object,
                    cli::HostOutput::Bitcode => luna::host::BinaryOutput::Bitcode,
                    _ => luna::host::BinaryOutput::Executable,
                };
                let bytes = luna::host::binary(&ir, &toolchain, kind)?;
                let publish = if kind == luna::host::BinaryOutput::Executable {
                    output::write_executable
                } else {
                    output::write_object
                };
                publish(
                    &options.input,
                    options.output.as_ref().ok_or("host output missing")?,
                    &bytes,
                )
            }
        }),
        Command::Help => output::write_stdout(cli::HELP),
        Command::Packages(request)
            if matches!(request.action, luna::package_cli::Action::Build { .. }) =>
        {
            request.build().and_then(|bytes| {
                let publish = if matches!(
                    request.action,
                    luna::package_cli::Action::Build {
                        output: luna::host::BinaryOutput::Executable,
                        ..
                    }
                ) {
                    output::write_executable
                } else {
                    output::write_object
                };
                publish(
                    &request.options.input,
                    request
                        .options
                        .output
                        .as_ref()
                        .ok_or("host output missing")?,
                    &bytes,
                )
            })
        }
        Command::Packages(request) => request.run().and_then(|dump| {
            if let Some(path) = &request.options.output {
                output::write_file(&request.options.input, path, &dump)
            } else {
                output::write_stdout(&dump)
            }
        }),
        Command::Invoke {
            options,
            entry,
            fuel,
            protocols,
        } => frontend::invoke_comptime(
            &options,
            &entry,
            fuel,
            protocols.as_ref().map(|(copy, _)| copy.as_str()),
            protocols.as_ref().map(|(_, drop)| drop.as_str()),
        )
        .map(|value| value.to_string())
        .and_then(|value| {
            if let Some(path) = &options.output {
                output::write_file(&options.input, path, &value)
            } else {
                output::write_stdout(&value)
            }
        }),
        Command::Test {
            host_fma,
            host_sqrt,
            host_copysign,
            host_intrinsics,
            timeout,
            optimization,
            native,
            list_only,
            filter,
            options,
            inline_functions,
            protocols,
            fuel,
        } => (if native {
            frontend::compile_bound_for_native_tests
        } else {
            frontend::compile_bound_with_host_protocols
        })(
            &options,
            protocols.as_ref().map(|(copy, _)| copy.as_str()),
            protocols.as_ref().map(|(_, drop)| drop.as_str()),
            host_fma.as_deref(),
            host_sqrt.as_deref(),
            host_copysign.as_deref(),
            &host_intrinsics,
        )
        .and_then(|(artifact, bindings)| {
            luna::package_cli::Action::Tests {
                host_fma,
                host_sqrt,
                host_copysign,
                host_intrinsics,
                timeout,
                optimization,
                native,
                list_only,
                filter,
                fuel,
                inline: inline_functions,
                protocols,
            }
            .run(artifact, &bindings, 0)
        })
        .and_then(|dump| {
            if let Some(path) = &options.output {
                output::write_file(&options.input, path, &dump)
            } else {
                output::write_stdout(&dump)
            }
        }),
        Command::Interpret {
            options,
            inline_functions,
            entry,
            protocols,
            fuel,
        } => frontend::compile_bound_with_resource_protocols(
            &options,
            protocols.as_ref().map(|(copy, _)| copy.as_str()),
            protocols.as_ref().map(|(_, drop)| drop.as_str()),
        )
        .and_then(|(artifact, bindings)| {
            luna::package_cli::Action::Interpret {
                entry,
                fuel,
                inline: inline_functions,
                protocols,
            }
            .run(artifact, &bindings, 0)
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
        Command::AnalyzeTextRegions {
            options,
            entry,
            fuel,
        } => frontend::compile(&options)
            .and_then(|artifact| {
                let frontend::Artifact::Nir(package) = artifact else {
                    return Err("text region analysis requires NIR".into());
                };
                luna::text_regions::analyze_in_package(&package, 0, &entry, fuel)
                    .map(|summary| summary.to_string())
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
        } => frontend::compile_bound_with_resource_protocols(
            &options,
            Some(&copy_protocol),
            drop_protocol.as_deref(),
        )
        .and_then(|(artifact, bindings)| {
            luna::package_cli::Action::Analyze {
                entry,
                fuel,
                copy: copy_protocol,
                drop: drop_protocol,
            }
            .run(artifact, &bindings, 0)
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
