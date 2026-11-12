//! Retain candidate-local descriptions without settling unknown components.
//! Copies are bounded before allocation; ownership does not authorize publication.
use super::{CallableContext, InputContext};
use nir::{Term, copy_budget::CopyBudgetExhausted as Error};

impl CallableContext {
    /// Retain a private checking description, charging its full copied payload.
    /// Failure does not settle or mutate the original candidate context.
    pub fn clone_bounded(&self, fuel: &mut usize) -> Result<Self, Error> {
        callable(self, fuel, 0)?;
        Ok(self.clone())
    }
}

impl InputContext {
    /// Copy a partial input context without converting unknowns into types.
    /// Depth and work exhaustion remain distinct from missing type information.
    pub fn clone_bounded(&self, fuel: &mut usize) -> Result<Self, Error> {
        input(self, fuel, 0)?;
        Ok(self.clone())
    }
}

fn charge(fuel: &mut usize, count: usize) -> Result<(), Error> {
    *fuel = fuel.checked_sub(count).ok_or(Error::Steps)?;
    Ok(())
}

fn enter(fuel: &mut usize, depth: usize) -> Result<(), Error> {
    if depth >= 128 {
        return Err(Error::Depth);
    }
    charge(fuel, 1)
}

fn term(value: &Term, fuel: &mut usize, depth: usize) -> Result<(), Error> {
    if depth >= 128 {
        return Err(Error::Depth);
    }
    nir::copy_budget::charge_term_copy(value, fuel, 128 - depth)
}

fn slots(values: &[Option<Term>], fuel: &mut usize, depth: usize) -> Result<(), Error> {
    charge(fuel, values.len())?;
    for value in values.iter().flatten() {
        term(value, fuel, depth)?;
    }
    Ok(())
}

fn callable(value: &CallableContext, fuel: &mut usize, depth: usize) -> Result<(), Error> {
    enter(fuel, depth)?;
    if let Some(environment) = &value.environment {
        slots(environment, fuel, depth + 1)?;
    }
    slots(&value.parameters, fuel, depth + 1)?;
    if let Some(result) = &value.result {
        term(result, fuel, depth + 1)?;
    }
    if let Some(result) = &value.result_context {
        input(result, fuel, depth + 1)?;
    }
    Ok(())
}

fn input(value: &InputContext, fuel: &mut usize, depth: usize) -> Result<(), Error> {
    enter(fuel, depth)?;
    match value {
        InputContext::Known(ty) => term(ty, fuel, depth + 1)?,
        InputContext::Optional(inner) => input(inner, fuel, depth + 1)?,
        InputContext::ErrorResult { errors, result } => {
            charge(fuel, errors.len())?;
            for error in errors {
                term(error, fuel, depth + 1)?;
            }
            input(result, fuel, depth + 1)?;
        }
        InputContext::Computation { effects, result } => {
            charge(fuel, effects.len())?;
            for effect in effects {
                charge(fuel, effect.arguments.len())?;
                for argument in &effect.arguments {
                    term(argument, fuel, depth + 1)?;
                }
            }
            input(result, fuel, depth + 1)?;
        }
        InputContext::Tuple(fields)
        | InputContext::Nominal {
            arguments: fields, ..
        } => {
            charge(fuel, fields.len())?;
            for field in fields {
                input(field, fuel, depth + 1)?;
            }
        }
        InputContext::NormalTuple {
            fields,
            list_domain,
            ..
        } => {
            if let Some(domain) = list_domain {
                term(domain, fuel, depth + 1)?;
            }
            charge(fuel, fields.len())?;
            for field in fields {
                input(field, fuel, depth + 1)?;
            }
        }
        InputContext::Record(fields) => {
            charge(fuel, fields.len())?;
            for (name, field) in fields {
                charge(fuel, name.len())?;
                input(field, fuel, depth + 1)?;
            }
        }
        InputContext::Callable(value) => callable(value, fuel, depth + 1)?,
        InputContext::Unknown
        | InputContext::UnknownValue
        | InputContext::UnknownDependentValue => {}
    }
    Ok(())
}
