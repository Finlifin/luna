//! Versioned process boundary for build tools supplying explicit source packages.
use crate::{
    cli::{self, Command, Emit, Options},
    frontend::Artifact,
    packages::SourcePackage,
};
use std::ffi::OsString;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub packages: Vec<SourcePackage>,
    pub dependencies: Vec<(u32, String, u32)>,
    pub primary: u32,
    pub builtin: Option<u32>,
    pub options: Options,
    pub action: Action,
    pub with_inputs: bool,
    /// Typed embedding input, included in the complete request cache key.
    /// None is unavailable; Some(empty) explicitly installs an empty snapshot.
    /// Process requests encode this with --package-metadata-v1.
    pub metadata: Option<Vec<crate::build_context::metadata::Input>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Compile,
    Build {
        output: crate::host::BinaryOutput,
        host_fma: Option<String>,
        host_sqrt: Option<String>,
        host_copysign: Option<String>,
        host_intrinsics: Vec<(nir::intrinsics::Operation, String)>,
        discard: bool,
        entry: String,
        fuel: u64,
        inline: bool,
        optimization: Option<crate::host::OptimizationLevel>,
        protocols: Option<(String, String)>,
    },
    EmitHost {
        assembly: bool,
        host_fma: Option<String>,
        host_sqrt: Option<String>,
        host_copysign: Option<String>,
        host_intrinsics: Vec<(nir::intrinsics::Operation, String)>,
        discard: bool,
        entry: String,
        fuel: u64,
        inline: bool,
        optimization: Option<crate::host::OptimizationLevel>,
        protocols: Option<(String, String)>,
    },
    Tests {
        host_fma: Option<String>,
        host_sqrt: Option<String>,
        host_copysign: Option<String>,
        host_intrinsics: Vec<(nir::intrinsics::Operation, String)>,
        timeout: std::time::Duration,
        optimization: Option<crate::host::OptimizationLevel>,
        native: bool,
        list_only: bool,
        filter: Option<String>,
        fuel: u64,
        inline: bool,
        protocols: Option<(String, String)>,
    },
    Invoke {
        protocols: Option<(String, String)>,
        entry: String,
        fuel: u64,
    },
    Instantiate {
        entry: String,
        fuel: u64,
    },
    Interpret {
        entry: String,
        fuel: u64,
        inline: bool,
        protocols: Option<(String, String)>,
    },
    AnalyzeTextRegions {
        entry: String,
        fuel: u64,
        inline: bool,
    },
    Analyze {
        entry: String,
        fuel: u64,
        copy: String,
        drop: Option<String>,
    },
}

fn take(args: &mut impl Iterator<Item = OsString>, what: &str) -> Result<OsString, String> {
    args.next()
        .ok_or_else(|| format!("missing {what} in --packages-v1 request"))
}
fn text(args: &mut impl Iterator<Item = OsString>, what: &str) -> Result<String, String> {
    take(args, what)?
        .into_string()
        .map_err(|_| format!("{what} must be UTF-8"))
}
fn number<T: std::str::FromStr>(
    args: &mut impl Iterator<Item = OsString>,
    what: &str,
) -> Result<T, String> {
    let value = text(args, what)?;
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("{what} must be an unsigned decimal integer"));
    }
    value.parse().map_err(|_| format!("{what} is out of range"))
}

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Request, String> {
    let mut args = args.into_iter();
    let mut packages = Vec::new();
    let mut dependencies = Vec::new();
    let mut primary = None;
    let mut builtin = None;
    let mut forwarded = Vec::new();
    let mut with_inputs = false;
    let mut metadata = None;
    while let Some(argument) = args.next() {
        match argument.to_str() {
            Some("--package-metadata-v1") => {
                if metadata.is_some() {
                    return Err("--package-metadata-v1 supplied more than once".into());
                }
                metadata = Some(crate::build_context::metadata::wire::decode_hex(&text(
                    &mut args,
                    "metadata payload",
                )?)?);
            }
            Some("--with-inputs") => {
                if std::mem::replace(&mut with_inputs, true) {
                    return Err("--with-inputs supplied more than once".into());
                }
            }
            Some("--package") => packages.push(SourcePackage {
                resolution_id: number(&mut args, "package resolution ID")?,
                identity: number(&mut args, "package semantic identity")?,
                name: text(&mut args, "package name")?,
                entry: take(&mut args, "package entry path")?.into(),
            }),
            Some("--dependency") => dependencies.push((
                number(&mut args, "requesting package ID")?,
                text(&mut args, "dependency alias")?,
                number(&mut args, "providing package ID")?,
            )),
            Some("--primary") => {
                if primary
                    .replace(number(&mut args, "primary package ID")?)
                    .is_some()
                {
                    return Err("--primary supplied more than once".into());
                }
            }
            Some("--builtin") => {
                if builtin
                    .replace(number(&mut args, "builtin package ID")?)
                    .is_some()
                {
                    return Err("--builtin supplied more than once".into());
                }
            }
            Some(
                "--discard-result" | "--inline-functions" | "--test" | "--test-native"
                | "--list-tests",
            ) => forwarded.push(argument),
            Some(
                flag @ ("--build-target" | "--package-build-target" | "--package-build-feature"),
            ) => {
                let count = match flag {
                    "--build-target" => 3,
                    "--package-build-target" => 4,
                    _ => 2,
                };
                forwarded.push(argument);
                for _ in 0..count {
                    forwarded.push(take(&mut args, "build target value")?);
                }
            }
            Some(
                "--build"
                | "--emit-llvm"
                | "--emit-asm"
                | "--emit-object"
                | "--emit-bitcode"
                | "--invoke-comptime"
                | "--instantiate"
                | "--interpret"
                | "--analyze-resources"
                | "--analyze-text-regions"
                | "--test-filter"
                | "--test-timeout-ms"
                | "--fuel"
                | "--build-feature"
                | "--build-mode"
                | "--package-build-mode"
                | "--host-fma"
                | "--host-sqrt"
                | "--host-copysign"
                | "--host-intrinsic"
                | "--comptime-intrinsic"
                | "--opt-level"
                | "--copy-protocol"
                | "--drop-protocol"
                | "--emit"
                | "--output"
                | "--splice-protocol"
                | "--meta-list-type"
                | "--tuple-type"
                | "--list-pattern-protocol"
                | "--index-protocol"
                | "--ord-protocol"
                | "--eq-protocol"
                | "--literal-into-protocol"
                | "--add-protocol"
                | "--sub-protocol"
                | "--mul-protocol"
                | "--div-protocol"
                | "--rem-protocol"
                | "--neg-protocol"
                | "--not-protocol",
            ) => {
                let value = take(&mut args, "option value")?;
                forwarded.extend([argument, value]);
            }
            _ => {
                return Err(format!(
                    "unsupported --packages-v1 argument: {}",
                    argument.to_string_lossy()
                ));
            }
        }
    }
    let primary = primary.ok_or("--packages-v1 requires --primary")?;
    let entry = packages
        .iter()
        .find(|p| p.resolution_id == primary)
        .ok_or("primary source package is absent")?
        .entry
        .clone();
    // Parse ordinary compile options once, preserving their duplicate checks
    // and source selector grammar. Paths remain OS strings throughout.
    let mut ordinary = forwarded;
    ordinary.push("--".into());
    ordinary.push(entry.into_os_string());
    let (options, action) = match cli::parse(ordinary)? {
        Command::Compile(options) => (options, Action::Compile),
        Command::Invoke {
            protocols,
            options,
            entry,
            fuel,
        } => (
            options,
            Action::Invoke {
                entry,
                fuel,
                protocols,
            },
        ),
        Command::Instantiate {
            options,
            entry,
            fuel,
        } => (options, Action::Instantiate { entry, fuel }),
        Command::Host {
            host_fma,
            host_sqrt,
            host_copysign,
            host_intrinsics,
            options,
            entry,
            fuel,
            output_kind:
                output_kind @ (crate::cli::HostOutput::Llvm | crate::cli::HostOutput::Assembly),
            discard_result,
            inline_functions,
            optimization,
            protocols,
        } => (
            options,
            Action::EmitHost {
                assembly: output_kind == crate::cli::HostOutput::Assembly,
                host_fma,
                host_sqrt,
                host_copysign,
                host_intrinsics,
                discard: discard_result,
                entry,
                fuel,
                inline: inline_functions,
                optimization,
                protocols,
            },
        ),
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
        } => (
            options,
            Action::Build {
                output: match output_kind {
                    crate::cli::HostOutput::Object => crate::host::BinaryOutput::Object,
                    crate::cli::HostOutput::Bitcode => crate::host::BinaryOutput::Bitcode,
                    _ => crate::host::BinaryOutput::Executable,
                },
                host_fma,
                host_sqrt,
                host_copysign,
                host_intrinsics,
                discard: discard_result,
                entry,
                fuel,
                inline: inline_functions,
                optimization,
                protocols,
            },
        ),
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
        } => (
            options,
            Action::Tests {
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
            },
        ),
        Command::Interpret {
            options,
            entry,
            fuel,
            inline_functions,
            protocols,
        } => (
            options,
            Action::Interpret {
                entry,
                fuel,
                inline: inline_functions,
                protocols,
            },
        ),
        Command::AnalyzeTextRegions {
            options,
            entry,
            fuel,
            inline_functions,
        } => (
            options,
            Action::AnalyzeTextRegions {
                entry,
                fuel,
                inline: inline_functions,
            },
        ),
        Command::AnalyzeResources {
            options,
            entry,
            fuel,
            copy_protocol,
            drop_protocol,
        } => (
            options,
            Action::Analyze {
                entry,
                fuel,
                copy: copy_protocol,
                drop: drop_protocol,
            },
        ),
        _ => return Err("--packages-v1 requires a compilation action".into()),
    };
    if with_inputs && matches!(action, Action::Build { .. }) {
        return Err("--with-inputs cannot wrap a native binary artifact; use --emit-llvm for an observed text artifact".into());
    }
    if options.emit == Emit::Ast {
        return Err("--packages-v1 supports --emit hir, nir or attributes".into());
    }
    Ok(Request {
        packages,
        dependencies,
        primary,
        builtin,
        options,
        action,
        with_inputs,
        metadata,
    })
}

impl Request {
    /// Execute the selected action using the semantic identity, never the
    /// request-local resolution ID or an assumed primary identity of zero.
    pub fn run(&self) -> Result<String, String> {
        if matches!(self.action, Action::Build { .. }) {
            return Err("native build returns bytes; use Request::build".into());
        }
        let mut inputs = crate::packages::CompilationInputs::default();
        let mut structural_fuel = 1_000_000;
        let mut invocation = match &self.action {
            Action::Invoke { entry, fuel, .. } => Some(crate::packages::BuilderInvocation {
                entry,
                structural_fuel: &mut structural_fuel,
                invocation_fuel: *fuel,
                value: None,
            }),
            _ => None,
        };
        let (artifact, bindings) = self.compile_observed_with_invocation(
            if self.with_inputs {
                Some(&mut inputs)
            } else {
                None
            },
            invocation.as_mut(),
        )?;
        let identity = self
            .packages
            .iter()
            .find(|p| p.resolution_id == self.primary)
            .ok_or("primary source package is absent")?
            .identity;
        let result = if let Some(invocation) = invocation {
            invocation
                .value
                .ok_or("comptime invocation requires NIR")?
                .to_string()
        } else {
            self.action.run(artifact, &bindings, identity)?
        };
        let result = if self.with_inputs && matches!(self.action, Action::EmitHost { .. }) {
            let kind = if matches!(self.action, Action::EmitHost { assembly: true, .. }) {
                "host-assembly"
            } else {
                "llvm-ir"
            };
            format!("({kind} {})", nir::program::Literal::String(result))
        } else {
            result
        };
        Ok(if self.with_inputs {
            format!("(compilation-result (version 1) (result {result}) {inputs})")
        } else {
            result
        })
    }

    /// Produce executable, object, or LLVM bitcode bytes without publishing them. All package
    /// sources are checked against the requested output during compilation.
    pub fn build(&self) -> Result<Vec<u8>, String> {
        let Action::Build {
            output,
            host_fma,
            host_sqrt,
            host_copysign,
            host_intrinsics,
            discard,
            entry,
            fuel,
            inline,
            optimization,
            protocols,
        } = &self.action
        else {
            return Err("Request::build requires a native build action".into());
        };
        if self.with_inputs {
            return Err("--with-inputs cannot wrap a native binary artifact".into());
        }
        let (artifact, bindings) = self.compile_bound()?;
        let identity = self
            .packages
            .iter()
            .find(|p| p.resolution_id == self.primary)
            .ok_or("primary source package is absent")?
            .identity;
        let ir = Action::EmitHost {
            assembly: false,
            host_fma: host_fma.clone(),
            host_sqrt: host_sqrt.clone(),
            host_copysign: host_copysign.clone(),
            host_intrinsics: host_intrinsics.clone(),
            discard: *discard,
            entry: entry.clone(),
            fuel: *fuel,
            inline: *inline,
            optimization: None,
            protocols: protocols.clone(),
        }
        .run(artifact, &bindings, identity)?;
        crate::host::binary(
            &ir,
            &crate::host::Toolchain {
                optimization: optimization.unwrap_or_default(),
                ..Default::default()
            },
            *output,
        )
    }

    pub fn compile(&self) -> Result<Artifact, String> {
        self.compile_bound().map(|(artifact, _)| artifact)
    }
    fn compile_bound(&self) -> Result<(Artifact, elaboration::program::Protocols), String> {
        self.compile_observed(None)
    }
    pub(crate) fn semantic_target(&self) -> Result<Option<nir::target::TargetData>, String> {
        let target = if matches!(&self.action, Action::Build { .. } | Action::EmitHost { .. }) {
            Some(
                crate::host::Toolchain::default()
                    .target_layout()?
                    .semantic_target()?,
            )
        } else if matches!(&self.action, Action::Tests { native: true, .. }) {
            crate::frontend::native_test_target()
        } else {
            None
        };
        let primary = self
            .packages
            .iter()
            .find(|package| package.resolution_id == self.primary)
            .ok_or("primary source package is absent")?
            .identity;
        self.options.build_targets.resolve_layout(primary, target)
    }

    pub(crate) fn compile_observed(
        &self,
        inputs: Option<&mut crate::packages::CompilationInputs>,
    ) -> Result<(Artifact, elaboration::program::Protocols), String> {
        self.compile_observed_with_invocation(inputs, None)
    }

    fn compile_observed_with_invocation(
        &self,
        inputs: Option<&mut crate::packages::CompilationInputs>,
        invocation: Option<&mut crate::packages::BuilderInvocation<'_>>,
    ) -> Result<(Artifact, elaboration::program::Protocols), String> {
        let dependencies = self
            .dependencies
            .iter()
            .map(|(requester, alias, provider)| resolve::DependencyEdge {
                requester: *requester,
                alias: alias.clone(),
                provider: *provider,
            })
            .collect::<Vec<_>>();
        let options = &self.options;
        options.build_modes.validate_stage(options.emit)?;
        options.build_targets.validate_stage(options.emit)?;
        let (copy, drop) = match &self.action {
            Action::Tests {
                protocols: Some((copy, drop)),
                ..
            }
            | Action::Build {
                protocols: Some((copy, drop)),
                ..
            }
            | Action::EmitHost {
                protocols: Some((copy, drop)),
                ..
            }
            | Action::Invoke {
                protocols: Some((copy, drop)),
                ..
            }
            | Action::Interpret {
                protocols: Some((copy, drop)),
                ..
            } => (Some(copy.as_str()), Some(drop.as_str())),
            Action::Analyze { copy, drop, .. } => (Some(copy.as_str()), drop.as_deref()),
            _ => (None, None),
        };
        let target = self.semantic_target()?;
        let primary_identity = self
            .packages
            .iter()
            .find(|package| package.resolution_id == self.primary)
            .ok_or("primary source package is absent")?
            .identity;
        let configured = options.build_modes.bind(
            self.packages.iter().map(|package| package.identity),
            primary_identity,
            &Default::default(),
        )?;
        let configured = options.build_targets.bind(
            self.packages.iter().map(|package| package.identity),
            primary_identity,
            &configured,
        )?;
        let mut configured = configured;
        if let Some(inputs) = &self.metadata {
            if options.emit != Emit::Nir {
                return Err("package metadata requires --emit nir".into());
            }
            // Charge descriptor copies before cloning input into the snapshot.
            // The same budget covers graph validation and field installation.
            let mut fuel = 100_000usize
                .checked_sub(inputs.len())
                .ok_or("package metadata input budget exceeded")?;
            for input in inputs {
                fuel = fuel
                    .checked_sub(input.fields.len())
                    .ok_or("package metadata input budget exceeded")?;
                for (name, entry) in &input.fields {
                    fuel = fuel
                        .checked_sub(name.len())
                        .ok_or("package metadata input budget exceeded")?;
                    for term in [entry.ty(), entry.value()] {
                        nir::type_computations::charge_term(term, &mut fuel)
                            .map_err(|_| "package metadata input budget exceeded")?;
                    }
                }
            }
            crate::build_context::metadata::Snapshot::new(
                crate::build_context::metadata::VERSION,
                &self.packages,
                &dependencies,
                inputs.clone(),
                &mut fuel,
            )
            .map_err(|error| error.to_string())?
            .configure(&mut configured);
        }
        crate::packages::compile_observed_for_target_with_invocation(
            &self.packages,
            &dependencies,
            self.primary,
            self.builtin,
            options.emit,
            options.output.as_deref(),
            &configured,
            crate::protocols::type_constructors::Selectors {
                comptime_intrinsics: &options.comptime_intrinsics,
                host_fma: match &self.action {
                    Action::Build { host_fma, .. }
                    | Action::EmitHost { host_fma, .. }
                    | Action::Tests { host_fma, .. } => host_fma.as_deref(),
                    _ => None,
                },
                host_sqrt: match &self.action {
                    Action::Build { host_sqrt, .. }
                    | Action::EmitHost { host_sqrt, .. }
                    | Action::Tests { host_sqrt, .. } => host_sqrt.as_deref(),
                    _ => None,
                },
                host_copysign: match &self.action {
                    Action::Build { host_copysign, .. }
                    | Action::EmitHost { host_copysign, .. }
                    | Action::Tests { host_copysign, .. } => host_copysign.as_deref(),
                    _ => None,
                },
                host_intrinsics: match &self.action {
                    Action::Build {
                        host_intrinsics, ..
                    }
                    | Action::EmitHost {
                        host_intrinsics, ..
                    }
                    | Action::Tests {
                        host_intrinsics, ..
                    } => host_intrinsics,
                    _ => &[],
                },
                copy,
                drop,
                splice: options.splice_protocol.as_deref(),
                meta_list: options.meta_list_type.as_deref(),
                tuple: options.tuple_type.as_deref(),
                list_pattern: options.list_pattern_protocol.as_deref(),
                iterator: options.iterator_protocol.as_deref(),
                into_iterator: options.into_iterator_protocol.as_deref(),
                index: options.index_protocol.as_deref(),
                ordering: options.ord_protocol.as_deref(),
                equality: options.eq_protocol.as_deref(),
                literal_into: options.literal_into_protocol.as_deref(),
                arithmetic: std::array::from_fn(|i| options.arithmetic_protocols[i].as_deref()),
                ..Default::default()
            },
            inputs,
            target,
            invocation,
        )
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn package_entry_os_paths_are_not_lossily_decoded() {
        let entry = OsString::from_vec(b"--source-\xff.fl".to_vec());
        let args = ["--package", "1", "18446744073709551615", "app"]
            .into_iter()
            .map(OsString::from)
            .chain([entry.clone()])
            .chain(
                ["--primary", "1", "--emit", "nir"]
                    .into_iter()
                    .map(OsString::from),
            );
        let request = parse(args).unwrap();
        assert_eq!(request.packages[0].identity, u64::MAX);
        assert_eq!(request.packages[0].entry.as_os_str(), entry.as_os_str());
        assert_eq!(request.options.input.as_os_str(), entry.as_os_str());
    }
}

impl Action {
    /// Apply an action to checked IR and its resolved language-item bindings.
    /// Names are used only by frontend selectors, never re-resolved in NIR.
    pub fn run(
        &self,
        artifact: Artifact,
        bindings: &elaboration::program::Protocols,
        identity: u64,
    ) -> Result<String, String> {
        if *self == Action::Compile {
            return Ok(artifact.to_string());
        }
        let Artifact::Nir(mut package) = artifact else {
            return Err("action requires NIR".into());
        };
        match self {
            Action::Compile => unreachable!(),
            Action::Build { .. } => Err("native build returns bytes; use Request::build".into()),
            Action::EmitHost {
                assembly,
                host_fma: _,
                host_sqrt: _,
                host_copysign: _,
                host_intrinsics: _,
                discard,
                entry,
                fuel,
                inline,
                optimization,
                protocols,
            } => {
                let selected =
                    if protocols.is_some() || bindings.copy.is_some() || bindings.drop.is_some() {
                        Some((
                            bindings.copy.ok_or("host Copy binding is missing")?,
                            bindings.drop.ok_or("host Drop binding is missing")?,
                        ))
                    } else {
                        None
                    };
                let ir = if *discard {
                    crate::host::llvm_ir_discarding_result(
                        &package,
                        Some(identity),
                        entry,
                        *fuel,
                        selected.ok_or("discard result requires Copy/Drop bindings")?,
                        *inline,
                    )?
                } else {
                    crate::host::llvm_ir_in_package(
                        &package, identity, entry, *fuel, selected, *inline,
                    )?
                };
                if *assembly {
                    crate::host::Toolchain {
                        optimization: optimization.unwrap_or_default(),
                        ..Default::default()
                    }
                    .assembly(&ir)
                } else if let Some(optimization) = optimization {
                    crate::host::Toolchain {
                        optimization: *optimization,
                        ..Default::default()
                    }
                    .optimized_ir(&ir)
                } else {
                    Ok(ir)
                }
            }

            Action::Tests {
                list_only: true,
                filter,
                ..
            } => crate::testing::discover(&package, identity, filter.as_deref())
                .map(|tests| tests.to_string()),
            Action::Tests {
                native: true,
                timeout,
                filter,
                fuel,
                protocols,
                optimization,
                inline,
                ..
            } => {
                let toolchain = crate::host::Toolchain {
                    optimization: optimization.unwrap_or_default(),
                    ..Default::default()
                };
                let report = crate::testing::run_native_report_filtered_with_inlining(
                    &package,
                    identity,
                    *fuel,
                    (protocols.is_some() || bindings.copy.is_some() || bindings.drop.is_some())
                        .then_some(bindings),
                    filter.as_deref(),
                    &toolchain,
                    *timeout,
                    *inline,
                )?;
                if report.is_success() {
                    Ok(report.to_string())
                } else {
                    Err(format!("source tests failed: {report}"))
                }
            }
            Action::Tests {
                filter,
                fuel,
                inline,
                protocols,
                ..
            } => crate::testing::run_filtered(
                &package,
                identity,
                *fuel,
                *inline,
                (protocols.is_some() || bindings.copy.is_some() || bindings.drop.is_some())
                    .then_some(bindings),
                filter.as_deref(),
            ),
            Action::Invoke {
                entry,
                fuel,
                protocols,
            } => {
                elaboration::verification::optimization::elide_proven_checks(
                    &mut package,
                    &mut elaboration::verification::Limits::default(),
                );
                crate::invocation::invoke_selected(
                    &package,
                    entry,
                    Some(identity),
                    if protocols.is_some() || bindings.copy.is_some() || bindings.drop.is_some() {
                        Some((
                            crate::resources::Selection::Identity(
                                bindings.copy.ok_or("missing Copy binding")?,
                            ),
                            crate::resources::Selection::Identity(
                                bindings.drop.ok_or("missing Drop binding")?,
                            ),
                        ))
                    } else {
                        None
                    },
                    *fuel,
                )
                .map(|v| v.to_string())
            }
            Action::Instantiate { entry, fuel } => {
                crate::instantiation::instantiate_in_package(&package, identity, entry, *fuel)
                    .map(|v| v.to_string())
            }
            Action::Interpret {
                entry,
                fuel,
                inline,
                protocols,
            } => {
                let value = match protocols {
                    _ if *inline
                        || protocols.is_some()
                        || bindings.copy.is_some()
                        || bindings.drop.is_some() =>
                    {
                        crate::execution::interpret_protocols_selected(
                            &package,
                            entry,
                            Some(identity),
                            crate::resources::Selection::Identity(
                                bindings.copy.ok_or("missing Copy binding")?,
                            ),
                            crate::resources::Selection::Identity(
                                bindings.drop.ok_or("missing Drop binding")?,
                            ),
                            *fuel,
                            *inline,
                        )
                    }
                    _ => crate::execution::interpret_in_package(&package, identity, entry, *fuel),
                };
                value.map(|v| v.to_string())
            }
            Action::AnalyzeTextRegions {
                entry,
                fuel,
                inline,
            } => (if *inline {
                crate::text_regions::analyze_inlined_in_package(package, identity, entry, *fuel)
            } else {
                crate::text_regions::analyze_in_package(&package, identity, entry, *fuel)
            })
            .map(|summary| summary.to_string()),
            Action::Analyze {
                entry, fuel, drop, ..
            } => crate::resources::analyze_selected(
                &package,
                entry,
                Some(identity),
                crate::resources::Selection::Identity(bindings.copy.ok_or("missing Copy binding")?),
                if drop.is_some() {
                    Some(crate::resources::Selection::Identity(
                        bindings.drop.ok_or("missing Drop binding")?,
                    ))
                } else {
                    None
                },
                *fuel,
            )
            .map(|v| v.to_string()),
        }
    }
}

// Keep observed-input envelopes valid S-expressions while raw dumps remain LLVM.
