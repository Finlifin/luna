//! Explicit concrete runtime entry selection; no implicit program invocation.
use nir::program::{FunctionPhase, Package};

pub fn instantiate(package: &Package, entry: &str, fuel: u64) -> Result<iir::Package, String> {
    instantiate_selected(package, entry, None, fuel)
}

pub fn instantiate_in_package(package: &Package, identity: u64, entry: &str, fuel: u64) -> Result<iir::Package, String> {
    instantiate_selected(package, entry, Some(identity), fuel)
}

fn instantiate_selected(package: &Package, entry: &str, identity: Option<u64>, fuel: u64) -> Result<iir::Package, String> {
    let mut candidates = package.functions.iter().filter(|f| f.name == entry && identity.is_none_or(|id| f.declaration.package == id));
    let function = candidates
        .next()
        .ok_or_else(|| format!("IIR entry `{entry}` was not found"))?;
    if candidates.next().is_some() {
        return Err(format!("IIR entry `{entry}` is ambiguous"));
    }
    if function.phase != FunctionPhase::Runtime {
        return Err("IIR CLI entry must be an ordinary runtime function".into());
    }
    if !function.type_parameters.is_empty() {
        return Err(
            "IIR CLI entry needs concrete type arguments; provide a nongeneric wrapper".into(),
        );
    }
    let steps = usize::try_from(fuel).map_err(|_| "IIR fuel does not fit the host work counter")?;
    elaboration::materialization::instantiate(
        package,
        &[iir::InstanceKey {
            declaration: function.declaration,
            arguments: vec![],
        }],
        Default::default(),
        elaboration::ptc::Budget {
            steps,
            ..Default::default()
        },
    )
}
