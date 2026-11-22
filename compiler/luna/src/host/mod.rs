//! Native host delivery from the concrete IIR graph.
//! LLVM is a target representation, never a replacement for Flurry type identity.
mod discard;
pub mod intrinsic;
mod llvm;
mod test_entry;

pub use discard::llvm_ir_discarding_result;
pub(crate) use test_entry::{
    describe_error_branch, llvm_ir_test_declaration, synthesize_error_test_adapter,
};

mod tools;

use nir::program::Package;
pub use tools::{OptimizationLevel, TargetLayout, Toolchain};

pub fn llvm_ir(package: &Package, entry: &str, fuel: u64) -> Result<String, String> {
    llvm_ir_selected(package, entry, fuel, None)
}

/// Native emission after source protocol selection and full cleanup acquisition.
/// Cleanup admission accepts only boundaries with implemented native lowering.
pub fn llvm_ir_with_protocols(
    package: &Package,
    entry: &str,
    fuel: u64,
    copy: nir::DeclarationId,
    drop: nir::DeclarationId,
) -> Result<String, String> {
    llvm_ir_selected(package, entry, fuel, Some((copy, drop)))
}

/// Expand eligible inline calls and handlers, then rebuild and validate all
/// resource schedules from the transformed source. Never reuse pre-pass cleanup.
pub fn llvm_ir_inlined_with_protocols(
    package: &Package,
    entry: &str,
    fuel: u64,
    copy: nir::DeclarationId,
    drop: nir::DeclarationId,
) -> Result<String, String> {
    let expanded = inline_package(package, fuel)?;
    let prepared = prepare_with_protocols(&expanded, entry, fuel, copy, drop)?;
    llvm_ir_prepared(&prepared, fuel)
}

pub(crate) fn inline_package(package: &Package, fuel: u64) -> Result<Package, String> {
    let mut steps = host_budget(fuel)?;
    let mut expanded = package.clone();
    elaboration::program::inlining::expand(&mut expanded, &mut steps)
        .map_err(|error| format!("cannot prepare optional host inlining: {error}"))?;
    Ok(expanded)
}

/// Host entry selection for an explicit semantic package identity.
pub fn llvm_ir_in_package(
    package: &Package,
    identity: u64,
    entry: &str,
    fuel: u64,
    protocols: Option<(nir::DeclarationId, nir::DeclarationId)>,
    inline: bool,
) -> Result<String, String> {
    if inline && protocols.is_none() {
        return Err("host inlining requires Copy and Drop bindings".into());
    }
    let expanded;
    let package = if inline {
        expanded = inline_package(package, fuel)?;
        &expanded
    } else {
        package
    };
    let (package, declaration) =
        crate::invocation::prepare_runtime_entry(package, entry, Some(identity), fuel)?;
    llvm_ir_declaration(&package, declaration, fuel, protocols)
}

/// Immutable checked handoff between source preparation and host lowering.
/// The constructor is private so an empty or edited schedule cannot bypass
/// resource admission. Inspection uses the existing S-expression display.
pub struct Prepared {
    attributes: nir::program::DefinitionAttributes,
    executable: crate::cleanup::Executable,
}

impl Prepared {
    pub fn graph(&self) -> &iir::Package {
        &self.executable.graph
    }

    pub fn schedule(&self) -> &iir::cleanup::Schedule {
        &self.executable.schedule
    }
}

impl std::fmt::Display for Prepared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.executable.fmt(f)
    }
}

/// Freeze a checked native input while retaining all cleanup contexts and sites.
/// This stage does not execute user code or require LLVM tools. Unsupported
/// native destructor lowering is diagnosed only by `llvm_ir_prepared`.
pub fn prepare_with_protocols(
    package: &Package,
    entry: &str,
    fuel: u64,
    copy: nir::DeclarationId,
    drop: nir::DeclarationId,
) -> Result<Prepared, String> {
    let (package, declaration) =
        crate::invocation::prepare_runtime_entry(package, entry, None, fuel)?;
    prepare_declaration_with_protocols(&package, declaration, fuel, copy, drop)
}

fn prepare_declaration_with_protocols(
    package: &Package,
    declaration: nir::DeclarationId,
    fuel: u64,
    copy: nir::DeclarationId,
    drop: nir::DeclarationId,
) -> Result<Prepared, String> {
    let mut steps = host_budget(fuel)?;
    let budget = elaboration::ptc::Budget {
        steps,
        ..Default::default()
    };
    let context = crate::resources::ProtocolContext::selected(
        &package,
        crate::resources::Selection::Identity(copy),
        Some(crate::resources::Selection::Identity(drop)),
        budget,
    )?;
    let executable = crate::cleanup::prepare_checked_source(
        &context,
        iir::InstanceKey {
            declaration,
            arguments: vec![].into(),
        },
        &mut steps,
        budget,
    )
    .map_err(|e| format!("cannot prepare resource-checked native compilation: {e}"))?;
    executable.graph.validate(&mut steps)?;
    Ok(Prepared {
        executable,
        attributes: package.definition_attributes.clone(),
    })
}

/// Lower exactly the graph/schedule pair produced by checked preparation.
/// Preserve the schedule until admission; never discard Invoke responsibilities.
pub fn llvm_ir_prepared(prepared: &Prepared, fuel: u64) -> Result<String, String> {
    llvm_ir_prepared_with_entry_mode(prepared, fuel, llvm::EntryMode::Display)
}

fn llvm_ir_prepared_with_entry_mode(
    prepared: &Prepared,
    fuel: u64,
    mode: llvm::EntryMode,
) -> Result<String, String> {
    let executable = &prepared.executable;
    let mut steps = host_budget(fuel)?;
    executable.graph.validate(&mut steps)?;
    llvm::validate_cleanup(&executable.graph, &executable.schedule, &mut steps)?;
    llvm::emit_with_entry_mode(
        &executable.graph,
        steps,
        true,
        Some(&executable.schedule),
        Some(&executable.entry),
        mode,
        &prepared.attributes,
    )
}

fn host_budget(fuel: u64) -> Result<usize, String> {
    let steps = usize::try_from(fuel).map_err(|_| "host pipeline budget is too large")?;
    if steps == 0 {
        return Err("host pipeline budget must be positive".into());
    }
    Ok(steps)
}

fn llvm_ir_selected(
    package: &Package,
    entry: &str,
    fuel: u64,
    protocols: Option<(nir::DeclarationId, nir::DeclarationId)>,
) -> Result<String, String> {
    if let Some((copy, drop)) = protocols {
        let executable = prepare_with_protocols(package, entry, fuel, copy, drop)?;
        return llvm_ir_prepared(&executable, fuel);
    }
    let (package, declaration) =
        crate::invocation::prepare_runtime_entry(package, entry, None, fuel)?;
    llvm_ir_declaration(&package, declaration, fuel, None)
}

/// Internal identity entry used by source tests; test labels are not names.
pub(crate) fn llvm_ir_declaration(
    package: &Package,
    declaration: nir::DeclarationId,
    fuel: u64,
    protocols: Option<(nir::DeclarationId, nir::DeclarationId)>,
) -> Result<String, String> {
    llvm_ir_declaration_with_entry_mode(
        package,
        declaration,
        fuel,
        protocols,
        llvm::EntryMode::Display,
    )
}

fn llvm_ir_declaration_with_entry_mode(
    package: &Package,
    declaration: nir::DeclarationId,
    fuel: u64,
    protocols: Option<(nir::DeclarationId, nir::DeclarationId)>,
    mode: llvm::EntryMode,
) -> Result<String, String> {
    if let Some((copy, drop)) = protocols {
        let prepared = prepare_declaration_with_protocols(package, declaration, fuel, copy, drop)?;
        return llvm_ir_prepared_with_entry_mode(&prepared, fuel, mode);
    }
    let steps = host_budget(fuel)?;
    let mut graph = elaboration::materialization::instantiate(
        &package,
        &[iir::InstanceKey {
            declaration,
            arguments: vec![].into(),
        }],
        Default::default(),
        elaboration::ptc::Budget {
            steps,
            ..Default::default()
        },
    )?;
    let mut validation_steps = steps;
    graph.validate(&mut validation_steps)?;
    let mut cert_fuel = steps.max(250_000).min(2_000_000);
    elaboration::verification::behavior::certify_iir_package(&mut graph, None, &mut cert_fuel);
    llvm::emit_with_entry_mode(
        &graph,
        steps,
        false,
        None,
        None,
        mode,
        &package.definition_attributes,
    )
}

/// Generate and verify LLVM, emit a native object, then link a complete executable.
/// The caller publishes the returned bytes atomically after source-output checks.
pub fn executable(ir: &str, toolchain: &Toolchain) -> Result<Vec<u8>, String> {
    toolchain.compile(ir)
}

/// Generate a verified, optimized native relocatable object without linking.
/// This uses the same host target checks as executable generation. The caller
/// owns publication and linking (including any required host runtime libraries).
/// Entry adapters and private function ABIs remain those of the supplied IR;
/// emitting an object does not turn internal Flurry functions into FFI exports.
pub fn object(ir: &str, toolchain: &Toolchain) -> Result<Vec<u8>, String> {
    toolchain.object(ir)
}

#[cfg(test)]
mod method_contract_tests;

/// Binary host delivery stages. Bitcode and objects do not invoke a linker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOutput {
    Executable,
    Object,
    Bitcode,
}

pub fn bitcode(ir: &str, toolchain: &Toolchain) -> Result<Vec<u8>, String> {
    toolchain.bitcode(ir)
}

pub fn binary(ir: &str, toolchain: &Toolchain, output: BinaryOutput) -> Result<Vec<u8>, String> {
    match output {
        BinaryOutput::Executable => executable(ir, toolchain),
        BinaryOutput::Object => object(ir, toolchain),
        BinaryOutput::Bitcode => bitcode(ir, toolchain),
    }
}
