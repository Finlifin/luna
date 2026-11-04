//! Compare admitted clause inputs without treating ordinary data as Type equations.
//! No invocation, implementation selection, or inferred binding is performed.
use crate::implementation::equivalent_domains;
use crate::ptc::Context;
use nir::NominalAdmissionError as Error;
use nir::Term;
use nir::type_computations::charge_term;

/// `false` denies contextual identity; it is not a proof of disequality.
/// All recursive comparison and proof work consumes the caller's allowance.
pub fn equivalent(
    context: &Context,
    left: &Term,
    right: &Term,
    max_depth: usize,
    scope: &[nir::Parameter],
    frames: &[Vec<nir::Sort>],
    fuel: &mut usize,
) -> Result<bool, Error> {
    charge_term(left, fuel)?;
    charge_term(right, fuel)?;
    if left == right {
        return Ok(true);
    }
    if left.sort() != nir::Sort::Type || right.sort() != nir::Sort::Type {
        // Ordinary data never becomes a PTC Type equality. Only Type leaves
        // of an otherwise identical value skeleton may use contextual evidence.
        if left.index_values_definitely_differ_budgeted(right, fuel)? {
            return Ok(false);
        }
        let Some(relations) = left.index_type_relations_budgeted(right, fuel)? else {
            return Err(Error::Unresolved(
                "input value identity is not established".into(),
            ));
        };
        for (left, right) in relations {
            if !equivalent(context, left, right, max_depth, scope, frames, fuel)? {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    let closed = left.check_scope(&[]).is_ok() && right.check_scope(&[]).is_ok();
    if scope.is_empty() || closed || context.scope() == frames {
        return equivalent_domains(context, left, right, max_depth, fuel);
    }
    // A complete caller telescope can be compared as a Type. An incomplete
    // runtime telescope must instead retain its original receiving context.
    for _ in 0..2 {
        for parameter in scope {
            charge_term(&parameter.ty, fuel)?;
        }
    }
    let scoped = |ty: &Term| Term::Function {
        parameters: scope.to_vec(),
        result: Box::new(ty.clone()),
    };
    equivalent_domains(context, &scoped(left), &scoped(right), max_depth, fuel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ptc::Formula;
    use nir::{DeclarationId, Primitive, Sort};

    fn rigid(local: u32, sort: Sort) -> Term {
        Term::Rigid {
            declaration: DeclarationId { package: 7, local },
            sort,
        }
    }

    fn compare(
        context: &Context,
        left: &Term,
        right: &Term,
        fuel: &mut usize,
    ) -> Result<bool, Error> {
        equivalent(context, left, right, 128, &[], context.scope(), fuel)
    }

    #[test]
    fn ordinary_values_use_only_their_type_leaves_as_propositions() {
        let a = rigid(1, Sort::Type);
        let b = rigid(2, Sort::Type);
        let mut context = Context::default();
        context.assume(Formula::equal(a.clone(), b.clone()));
        let left = Term::TupleValue(vec![a, Term::Bool(true)]);
        let right = Term::TupleValue(vec![b.clone(), Term::Bool(true)]);
        assert!(compare(&context, &left, &right, &mut 100_000).unwrap());
        let different = Term::TupleValue(vec![b, Term::Bool(false)]);
        assert!(!compare(&context, &left, &different, &mut 100_000).unwrap());
        assert!(!matches!(
            compare(&Context::default(), &left, &right, &mut 100_000),
            Ok(true)
        ));
        assert!(matches!(
            compare(
                &context,
                &rigid(3, Sort::ComptimeValue),
                &rigid(4, Sort::ComptimeValue),
                &mut 100_000
            ),
            Err(Error::Unresolved(_))
        ));
    }

    #[test]
    fn input_identity_failure_consumes_fuel_and_can_retry() {
        let a = rigid(1, Sort::Type);
        let b = rigid(2, Sort::Type);
        let mut context = Context::default();
        context.assume(Formula::equal(a.clone(), b.clone()));
        let mut fuel = 2;
        assert!(matches!(
            compare(&context, &a, &b, &mut fuel),
            Err(Error::Budget)
        ));
        assert_eq!(fuel, 0);
        fuel = 100_000;
        assert!(compare(&context, &a, &b, &mut fuel).unwrap());
        assert!(fuel < 100_000);
        let payload = Term::Symbol("wide".repeat(4096).into());
        let mut fuel = 8;
        assert!(matches!(
            compare(&context, &payload, &payload, &mut fuel),
            Err(Error::Budget)
        ));
        assert!(compare(&context, &payload, &payload, &mut 100_000).unwrap());
    }

    #[test]
    fn malformed_type_comparison_is_not_a_negative_identity_result() {
        let unbound = Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        };
        let primitive = Term::Primitive(Primitive::Bool);
        assert!(matches!(
            compare(&Context::default(), &unbound, &primitive, &mut 100_000),
            Err(Error::Invalid(_))
        ));
    }
}
