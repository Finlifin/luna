//! Preparation dependencies shared by phase validation and materialization.
use crate::program::{Expr, ExprKind as K, LocalId, Pattern};
use crate::{Sort, Term};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Budget,
    Invalid(&'static str),
}
impl Error {
    pub fn message(self) -> &'static str {
        match self {
            Self::Budget => "preparation dependency budget exhausted",
            Self::Invalid(message) => message,
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}
impl std::error::Error for Error {}
impl From<crate::visit::Error> for Error {
    fn from(error: crate::visit::Error) -> Self {
        match error {
            crate::visit::Error::Budget(_) => Self::Budget,
            crate::visit::Error::Invalid(message) => Self::Invalid(message),
        }
    }
}
impl From<Error> for String {
    fn from(error: Error) -> Self {
        error.to_string()
    }
}
fn charge(fuel: &mut usize, count: usize) -> Result<(), Error> {
    *fuel = fuel.checked_sub(count).ok_or(Error::Budget)?;
    Ok(())
}
pub fn references(expr: &Expr, fuel: &mut usize) -> Result<BTreeSet<LocalId>, Error> {
    let mut result = BTreeSet::new();
    crate::visit::expression_checked(expr, fuel, |expr, _| {
        match &expr.kind {
            K::Local(binding) => {
                result.insert(*binding);
            }
            K::Closure { captures, .. } => {
                for capture in captures {
                    if capture.value.is_none() {
                        result.insert(capture.binding);
                    }
                }
            }
            K::Assign { place, .. } | K::BitAssign { place, .. } | K::BitUpdate { place, .. } => {
                result.insert(place.root);
            }
            _ => {}
        }
        Ok::<_, Error>(())
    })?;
    Ok(result)
}

pub fn inputs(
    expr: &Expr,
    fuel: &mut usize,
) -> Result<Option<(BTreeSet<usize>, Vec<(usize, LocalId, Term)>)>, Error> {
    inputs_with_layouts(&[], expr, fuel)
}

/// Select tuple producers using validated Tuple-role descriptors, preserving
/// the normal-form type of their values rather than rewriting it structurally.
pub fn inputs_with_layouts(
    heads: &[crate::normal_forms::Head],
    expr: &Expr,
    fuel: &mut usize,
) -> Result<Option<(BTreeSet<usize>, Vec<(usize, LocalId, Term)>)>, Error> {
    let K::CallPreparation {
        statements,
        result: Some(call),
    } = &expr.kind
    else {
        return Ok(None);
    };
    let K::Call {
        callee, arguments, ..
    } = &call.kind
    else {
        return Ok(None);
    };
    let Term::Function { parameters, .. } = callee.ty.callable_signature() else {
        return Ok(None);
    };
    let mut needed = BTreeSet::new();
    for argument in arguments {
        let parameter = parameters
            .get(argument.parameter)
            .ok_or(Error::Invalid("preparation parameter index out of range"))?;
        if parameter.sort != Sort::Runtime && !argument.is_default {
            needed.extend(references(&argument.value, fuel)?);
        }
    }
    if needed.is_empty() {
        return Ok(None);
    };
    let mut owners = BTreeMap::new();
    let mut bindings = Vec::new();
    for (index, statement) in statements.iter().enumerate() {
        charge(fuel, 1)?;
        let mut fields = Vec::new();
        match &statement.kind {
            K::Let { binding, value } => fields.push((*binding, value.ty.clone())),
            K::LetPattern {
                pattern: pattern @ (Pattern::Tuple(_) | Pattern::Binding { .. } | Pattern::Wild),
                value,
                guard: None,
                otherwise: None,
            } => {
                let Some(selected) = pattern_bindings(heads, pattern, &value.ty, fuel)? else {
                    continue;
                };
                fields = selected;
            }
            _ => continue,
        }
        for (binding, ty) in fields {
            if owners.insert(binding, index).is_some() {
                return Err(Error::Invalid("duplicate preparation binding"));
            }
            bindings.push((index, binding, ty));
        }
    }
    let mut indices = BTreeSet::new();
    while let Some(binding) = needed.pop_first() {
        charge(fuel, 1)?;
        let Some(&index) = owners.get(&binding) else {
            continue;
        };
        if !indices.insert(index) {
            continue;
        }
        for reference in references(&statements[index], fuel)? {
            if let Some(&producer) = owners.get(&reference) {
                if producer == index {
                    continue;
                }
                if producer > index {
                    return Err(Error::Invalid("preparation reads a later producer"));
                }
                needed.insert(reference);
            }
        }
    }
    if indices.is_empty() {
        return Ok(None);
    };
    // Export only values used outside the selected computation. Intermediate
    // closures or aggregates can stay private to the comptime invocation.
    let mut exported = references(call, fuel)?;
    for (index, statement) in statements.iter().enumerate() {
        if !indices.contains(&index) {
            exported.extend(references(statement, fuel)?);
        }
    }
    bindings.retain(|(index, binding, _)| indices.contains(index) && exported.contains(binding));
    Ok(Some((indices, bindings)))
}

// Only irrefutable binding structure participates in dependency selection.
// Tuple roles expose field types without changing the producer's Type identity.
pub(crate) fn pattern_bindings(
    heads: &[crate::normal_forms::Head],
    pattern: &Pattern,
    ty: &Term,
    fuel: &mut usize,
) -> Result<Option<Vec<(LocalId, Term)>>, Error> {
    let mut pending = vec![(pattern, ty)];
    let mut bindings = Vec::new();
    while let Some((pattern, ty)) = pending.pop() {
        charge(fuel, 1)?;
        match pattern {
            Pattern::Wild => {}
            Pattern::Binding { binding, inner } => {
                bindings.push((*binding, ty.clone()));
                pending.push((inner, ty));
            }
            Pattern::Tuple(patterns) => {
                if *ty == Term::Primitive(crate::Primitive::Never) {
                    // The producer exits before unpacking. No tuple layout is
                    // required, but still visit the binding structure so phase
                    // validation can reject duplicates and unsupported patterns.
                    pending.extend(patterns.iter().rev().map(|pattern| (pattern, ty)));
                    continue;
                }
                let types = if let Term::Tuple(types) = ty {
                    types.as_slice()
                } else {
                    charge(fuel, heads.len())?;
                    let Some(types) = heads.iter().find_map(|head| head.tuple_elements(ty)) else {
                        return Ok(None);
                    };
                    types
                };
                if patterns.len() != types.len() {
                    return Err(Error::Invalid("preparation tuple arity mismatch"));
                }
                pending.extend(patterns.iter().zip(types).rev());
            }
            _ => return Err(Error::Invalid("unsupported preparation subpattern")),
        }
    }
    Ok(Some(bindings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preparation_limits_remain_distinct_from_invalid_binding_shapes() {
        let flag = Term::Primitive(crate::Primitive::Bool);
        let tuple = Term::Tuple(vec![flag.clone(); 2]);
        let valid = Pattern::Tuple(vec![Pattern::Wild; 2]);
        assert_eq!(
            pattern_bindings(&[], &valid, &tuple, &mut 0),
            Err(Error::Budget)
        );
        assert_eq!(
            pattern_bindings(&[], &valid, &tuple, &mut 3),
            Ok(Some(vec![]))
        );
        let invalid = Pattern::Tuple(vec![Pattern::Wild]);
        assert_eq!(
            pattern_bindings(&[], &invalid, &tuple, &mut 10),
            Err(Error::Invalid("preparation tuple arity mismatch"))
        );
        let body = Expr {
            source: crate::program::SourceRange { start: 0, end: 1 },
            ty: flag,
            kind: K::Local(LocalId { owner: 1, index: 0 }),
        };
        assert_eq!(references(&body, &mut 0), Err(Error::Budget));
        assert_eq!(references(&body, &mut 1).unwrap().len(), 1);
    }
}
