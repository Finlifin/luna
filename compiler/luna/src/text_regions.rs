//! Source entry selection for text dependencies, not memory-safety certification.
use elaboration::verification::memory::{TextRegionSummary, infer_text_regions};
use nir::program::{FunctionPhase, Package};

/// Select a concrete runtime entry in the primary semantic package. Parameters
/// remain symbolic caller inputs; this analysis does not invoke the entry.
pub fn analyze_in_package(
    package: &Package,
    identity: u64,
    entry: &str,
    fuel: u64,
) -> Result<TextRegionSummary, String> {
    let declaration = select_entry(package, identity, entry)?;
    let mut fuel = analysis_fuel(fuel)?;
    infer_text_regions(package, declaration, &[], &mut fuel)
}

/// Joint inline expansion and dependency inference share one work allowance.
/// Ownership keeps intermediate transformations private even when work fails.
pub fn analyze_inlined_in_package(
    mut package: Package,
    identity: u64,
    entry: &str,
    fuel: u64,
) -> Result<TextRegionSummary, String> {
    let declaration = select_entry(&package, identity, entry)?;
    let mut fuel = analysis_fuel(fuel)?;
    elaboration::program::inlining::expand(&mut package, &mut fuel)?;
    infer_text_regions(&package, declaration, &[], &mut fuel)
}

fn select_entry(
    package: &Package,
    identity: u64,
    entry: &str,
) -> Result<nir::DeclarationId, String> {
    let mut matches = package
        .functions
        .iter()
        .filter(|function| function.name == entry && function.declaration.package == identity);
    let function = matches
        .next()
        .ok_or_else(|| format!("text region entry `{entry}` was not found"))?;
    if matches.next().is_some() {
        return Err(format!("text region entry `{entry}` is ambiguous"));
    }
    if function.phase != FunctionPhase::Runtime {
        return Err("text region entry must be an ordinary runtime function".into());
    }
    if !function.type_parameters.is_empty()
        || !function.requirements.is_empty()
        || !function.capabilities.is_empty()
    {
        return Err("text region entry needs a nongeneric wrapper without unresolved type/capability assumptions".into());
    }
    Ok(function.declaration)
}

fn analysis_fuel(fuel: u64) -> Result<usize, String> {
    let fuel =
        usize::try_from(fuel).map_err(|_| "text region fuel does not fit the host work counter")?;
    if fuel == 0 {
        return Err("text region fuel must be positive".into());
    }
    Ok(fuel)
}
