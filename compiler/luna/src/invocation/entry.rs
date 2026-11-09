//! Host entry selection and bounded ordinary default-call preparation.
use super::*;
use std::sync::Arc;

pub(super) struct Entry {
    pub(super) declaration: DeclarationId,
    defaults: bool,
    call: Option<Arc<nir::program::Expr>>,
}
impl Entry {
    pub(super) fn new(function: &nir::program::Function) -> Self {
        Self {
            declaration: function.declaration,
            defaults: !function.parameters.is_empty(),
            call: None,
        }
    }

    pub(super) fn call(
        &mut self,
        package: &Package,
        fuel: &mut u64,
    ) -> Result<Option<Arc<nir::program::Expr>>, String> {
        if !self.defaults {
            return Ok(None);
        }
        *fuel = fuel
            .checked_sub(1)
            .ok_or("entry call preparation budget exhausted")?;
        if let Some(call) = &self.call {
            return Ok(Some(call.clone()));
        }
        let call = Arc::new(prepare_call(package, self.declaration, fuel)?);
        self.call = Some(call.clone());
        Ok(Some(call))
    }
}

pub(super) fn select<'a>(
    package: &'a Package,
    entry: &str,
    phase: FunctionPhase,
    identity: Option<u64>,
) -> Result<&'a nir::program::Function, String> {
    let mut matches = package.functions.iter().filter(|function| {
        function.name == entry && identity.is_none_or(|id| function.declaration.package == id)
    });
    let function = matches
        .next()
        .ok_or_else(|| format!("no function named '{entry}'"))?;
    if matches.next().is_some() {
        return Err(format!(
            "function name '{entry}' is ambiguous; use a uniquely named wrapper"
        ));
    }
    if function.phase != phase {
        return Err(format!(
            "'{entry}' is not a {} function",
            if phase == FunctionPhase::Comptime {
                "comptime"
            } else {
                "runtime"
            }
        ));
    }
    if !function.type_parameters.is_empty() {
        return Err(format!(
            "'{entry}' requires generic arguments; use a concrete wrapper"
        ));
    }
    if (0..function.parameters.len())
        .any(|index| !function.defaults.iter().any(|(slot, _)| *slot == index))
    {
        return Err(format!(
            "'{entry}' requires arguments; use a zero-argument wrapper"
        ));
    }
    Ok(function)
}

fn prepare_call(
    package: &Package,
    declaration: DeclarationId,
    fuel: &mut u64,
) -> Result<nir::program::Expr, String> {
    use nir::program::{CallArgument, Expr, ExprKind};
    *fuel = fuel
        .checked_sub(package.functions.len() as u64)
        .ok_or("entry declaration lookup budget exhausted")?;
    let function = package
        .functions
        .iter()
        .find(|function| function.declaration == declaration)
        .ok_or("entry declaration disappeared")?;
    if function.phase != FunctionPhase::Comptime || !function.type_parameters.is_empty() {
        return Err("entry call requires a concrete comptime declaration".into());
    }
    let Term::Function { result, .. } = &function.signature else {
        return Err("invalid invocation signature".into());
    };
    let mut steps =
        usize::try_from(*fuel).map_err(|_| "entry fuel does not fit the host counter")?;
    let before = steps;
    let copied = (|| {
        use nir::copy_budget::{CopyBudgetExhausted as E, charge_type_substitution};
        steps = steps
            .checked_sub(
                function
                    .parameters
                    .len()
                    .saturating_mul(2)
                    .saturating_add(function.defaults.len()),
            )
            .ok_or(E::Steps)?;
        charge_type_substitution(&function.signature, &[], &[], &mut steps)?;
        charge_type_substitution(result, &[], &[], &mut steps)?;
        for (_, value) in &function.defaults {
            let mut work = steps as u64;
            let result = nir::copy_budget::charge_expression(value, &mut work);
            steps = work as usize;
            result?;
        }
        Ok::<_, E>(())
    })();
    *fuel -= (before - steps) as u64;
    copied.map_err(|error| format!("entry call copy budget exhausted: {error:?}"))?;
    let mut defaults = vec![None; function.parameters.len()];
    for (slot, value) in &function.defaults {
        let target = defaults
            .get_mut(*slot)
            .ok_or("invalid entry default parameter")?;
        if target.replace(value).is_some() {
            return Err("duplicate entry default parameter".into());
        }
    }
    if defaults.iter().any(Option::is_none) {
        return Err("entry requires an explicit argument".into());
    }
    let source = function.body.source;
    Ok(Expr {
        source,
        ty: (**result).clone(),
        kind: ExprKind::Call {
            callee: Box::new(Expr {
                source,
                ty: function.callable_type(),
                kind: ExprKind::Function(declaration),
            }),
            parameter_bindings: function.parameters.clone(),
            arguments: defaults
                .into_iter()
                .enumerate()
                .map(|(parameter, value)| CallArgument {
                    passing: Default::default(),
                    parameter,
                    is_default: true,
                    value: value.unwrap().clone(),
                })
                .collect(),
        },
    })
}
