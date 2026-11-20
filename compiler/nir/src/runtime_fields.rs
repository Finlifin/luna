//! Resolve the domain of a field address from declaration evidence. The caller
//! supplies the root's lifted domain and the applicable struct catalog.
use crate::{NominalAdmissionError as Error, Term};

pub fn domain(
    value: &Term,
    fuel: &mut usize,
    root_domain: impl FnOnce(&Term, &mut usize) -> Result<Option<Term>, Error>,
    mut struct_field: impl FnMut(&Term, usize, &mut usize) -> Result<Term, Error>,
) -> Result<Option<Term>, Error> {
    let (root, fields) = path(value, fuel)?;
    if root.sort() != crate::Sort::Runtime {
        return Err("field address requires a runtime identity".into());
    }
    let Some(domain) = root_domain(root, fuel)? else {
        return Ok(None);
    };
    let Term::Lift(payload) = domain else {
        return Err("runtime projection requires a lifted root".into());
    };
    let mut payload = *payload;
    for index in fields {
        let Term::Pointer(target) = payload else {
            return Err("field address requires a pointer".into());
        };
        let field = match *target {
            Term::Record(mut fields) if index < fields.len() => fields.swap_remove(index).1,
            Term::Nominal { .. } => struct_field(&target, index, fuel)?,
            _ => return Err("runtime field outside record layout".into()),
        };
        payload = Term::Pointer(Box::new(field));
    }
    Ok(Some(Term::Lift(Box::new(payload))))
}

/// Decompose an identity into its supplied root and declaration-order selections.
pub fn path<'a>(value: &'a Term, fuel: &mut usize) -> Result<(&'a Term, Vec<usize>), Error> {
    let mut root = value;
    let mut fields = Vec::new();
    while let Term::RuntimeField { base, index } = root {
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
        if fields.len() >= 128 {
            return Err(Error::Budget);
        }
        fields.push(*index);
        root = base;
    }
    fields.reverse();
    Ok((root, fields))
}

/// Unequal closed indices denote distinct dependency identities. An open root
/// may later be substituted by a concrete root, so it supplies no apartness.
pub fn distinct(left: &Term, right: &Term, fuel: &mut usize) -> Result<bool, Error> {
    *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
    let (left, a) = path(left, fuel)?;
    let (right, b) = path(right, fuel)?;
    Ok(matches!((left, right), (Term::Runtime(_), Term::Runtime(_))) && (left != right || a != b))
}

/// Resolve an identity against the same nominal and binder world that admits
/// its surrounding type. Newtype representations are not selectable fields.
pub fn declared_domain(
    value: &Term,
    parameters: &std::collections::BTreeMap<crate::DeclarationId, Term>,
    runtimes: &std::collections::BTreeMap<crate::RuntimeId, Term>,
    package: Option<&crate::program::Package>,
    frames: &[Vec<crate::Sort>],
    fuel: &mut usize,
) -> Result<Option<Term>, Error> {
    domain(
        value,
        fuel,
        |root, fuel| {
            let source = match root {
                Term::Runtime(id) => runtimes.get(id),
                Term::Rigid {
                    declaration,
                    sort: crate::Sort::Runtime,
                } => parameters.get(declaration),
                _ => None,
            };
            source
                .map(|ty| {
                    crate::type_computations::charge_term(ty, fuel)?;
                    Ok(ty.clone())
                })
                .transpose()
        },
        |target, index, fuel| struct_field(target, index, package, frames, fuel),
    )
}

pub fn struct_field(
    target: &Term,
    index: usize,
    package: Option<&crate::program::Package>,
    frames: &[Vec<crate::Sort>],
    fuel: &mut usize,
) -> Result<Term, Error> {
    let Term::Nominal {
        declaration,
        arguments,
    } = target
    else {
        return Err("runtime field requires a struct".into());
    };
    let package = package.ok_or("runtime field requires a declaration catalog")?;
    *fuel = fuel
        .checked_sub(package.structs.len())
        .ok_or(Error::Budget)?;
    let mut found = package
        .structs
        .iter()
        .filter(|def| def.declaration == *declaration);
    let def = found
        .next()
        .ok_or("missing struct layout for runtime field")?;
    if found.next().is_some() {
        return Err("ambiguous struct layout for runtime field".into());
    }
    def.addressed_field(index, arguments, frames, fuel)
}

/// Reconstruct an address from a supplied root using the same field domains
/// used for identity admission. This emits projections without reading fields.
pub fn address(
    mut base: crate::program::Expr,
    fields: &[usize],
    source: crate::program::SourceRange,
    fuel: &mut usize,
    mut nominal: impl FnMut(&Term, usize, &mut usize) -> Result<Term, Error>,
) -> Result<crate::program::Expr, Error> {
    use crate::program::{Expr, ExprKind as K};
    for &index in fields {
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
        let Term::Pointer(target) = &base.ty else {
            return Err("field address requires a pointer".into());
        };
        let (ty, kind) = match target.as_ref() {
            Term::Record(fields) => {
                let (name, ty) = fields.get(index).ok_or("invalid record field index")?;
                crate::type_computations::charge_term(ty, fuel)?;
                let ty = ty.clone();
                let name = name.clone();
                (
                    ty,
                    K::PointerAnonymousField {
                        pointer: Box::new(base),
                        name,
                        index,
                    },
                )
            }
            Term::Nominal { declaration, .. } => {
                let declaration = *declaration;
                let ty = nominal(target, index, fuel)?;
                (
                    ty,
                    K::PointerField {
                        pointer: Box::new(base),
                        declaration,
                        index,
                    },
                )
            }
            _ => return Err("field address requires a record or struct".into()),
        };
        base = Expr {
            ty: Term::Pointer(Box::new(ty)),
            source,
            kind,
        };
    }
    Ok(base)
}
