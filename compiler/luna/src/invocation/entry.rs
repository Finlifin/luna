//! Host entry selection and bounded ordinary default-call preparation.
use super::*;
use nir::program::{CallArgument, Expr, ExprKind};
use std::sync::Arc;

pub(super) struct Entry {
    pub(super) declaration: DeclarationId,
    /// Index in the immutable package from which selection was made.
    function_index: usize,
    /// Declaration default indices, ordered by receiving parameter slot.
    default_order: Vec<usize>,
    call: Option<Arc<Expr>>,
}

pub(super) fn select_in(
    package: &Package,
    entry: &str,
    phase: FunctionPhase,
    identity: Option<u64>,
    fuel: &mut u64,
) -> Result<Entry, String> {
    let mut selected = None;
    for (index, function) in package.functions.iter().enumerate() {
        spend(fuel, 1)?;
        if identity.is_some_and(|id| function.declaration.package != id)
            || function.name.len() != entry.len()
        {
            continue;
        }
        spend(fuel, entry.len())?;
        if function.name == entry {
            if selected.replace(index).is_some() {
                return Err(format!(
                    "function name '{entry}' is ambiguous; use a uniquely named wrapper"
                ));
            }
        }
    }
    let function_index = selected.ok_or_else(|| format!("no function named '{entry}'"))?;
    let function = &package.functions[function_index].interface;
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
    spend(fuel, function.parameters.len().saturating_mul(2))?;
    spend(fuel, function.defaults.len())?;
    let mut supplied = vec![None; function.parameters.len()];
    for (index, (slot, _)) in function.defaults.iter().enumerate() {
        let slot = supplied
            .get_mut(*slot)
            .ok_or("invalid entry default parameter")?;
        if slot.replace(index).is_some() {
            return Err("duplicate entry default parameter".into());
        }
    }
    let default_order = supplied
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| format!("'{entry}' requires arguments; use a zero-argument wrapper"))?;
    Ok(Entry {
        declaration: function.declaration,
        function_index,
        default_order,
        call: None,
    })
}

fn spend(fuel: &mut u64, amount: usize) -> Result<(), String> {
    *fuel = fuel
        .checked_sub(amount as u64)
        .ok_or("entry selection budget exhausted")?;
    Ok(())
}

impl Entry {
    pub(super) fn call(
        &mut self,
        package: &Package,
        fuel: &mut u64,
    ) -> Result<Option<Arc<Expr>>, String> {
        if !self.has_defaults() {
            return Ok(None);
        }
        *fuel = fuel
            .checked_sub(1)
            .ok_or("entry call preparation budget exhausted")?;
        if let Some(call) = &self.call {
            return Ok(Some(call.clone()));
        }
        let call = Arc::new(self.default_call(package, fuel)?);
        self.call = Some(call.clone());
        Ok(Some(call))
    }

    pub(super) fn has_defaults(&self) -> bool {
        !self.default_order.is_empty()
    }

    pub(super) fn default_call(&self, package: &Package, fuel: &mut u64) -> Result<Expr, String> {
        let function = &package.functions[self.function_index].interface;
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
        let source = function.source;
        Ok(Expr {
            source,
            ty: (**result).clone(),
            kind: ExprKind::Call {
                callee: Box::new(Expr {
                    source,
                    ty: function.callable_type(),
                    kind: ExprKind::Function(self.declaration),
                }),
                parameter_bindings: function.parameters.clone(),
                arguments: self
                    .default_order
                    .iter()
                    .enumerate()
                    .map(|(parameter, index)| CallArgument {
                        passing: Default::default(),
                        parameter,
                        is_default: true,
                        value: function.defaults[*index].1.clone(),
                    })
                    .collect(),
            },
        })
    }
}
