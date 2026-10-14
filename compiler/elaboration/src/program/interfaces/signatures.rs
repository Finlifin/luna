//! Structural signature equality under a lexical type contract. Each leaf is
//! proved by PTC in its exact scope; function binders lift outer assumptions.
use crate::ptc::{Budget, Context, Formula, Outcome};
use nir::{Sort, Term};

pub(super) fn equivalent<'a>(
    context: &Context,
    pairs: impl IntoIterator<Item = (&'a Term, &'a Term)>,
    budget: Budget,
) -> bool {
    let mut fuel = budget.steps;
    for assumption in context.assumptions() {
        if !crate::ptc::within_limits(assumption, budget.max_depth.min(64), &mut fuel) {
            return false;
        }
    }
    for (left, right) in pairs {
        if crate::associated::charge_projection_copy(left, &mut fuel).is_err()
            || crate::associated::charge_projection_copy(right, &mut fuel).is_err()
            || left.check_scope(context.scope()).is_err()
            || right.check_scope(context.scope()).is_err()
            || !equal(context, left, right, budget, &mut fuel, 0).unwrap_or(false)
        {
            return false;
        }
    }
    true
}

fn charge(fuel: &mut usize) -> Option<()> {
    *fuel = fuel.checked_sub(1)?;
    Some(())
}
fn lift(formula: &Formula, fuel: &mut usize, depth: usize) -> Option<Formula> {
    charge(fuel)?;
    if depth > 64 {
        return None;
    }
    let mut term = |ty: &Term| {
        crate::associated::charge_projection_copy(ty, fuel).ok()?;
        ty.abstract_parameters(&[]).ok()
    };
    Some(match formula {
        Formula::Equal(crate::ptc::Equality(a, b)) => Formula::equal(term(a)?, term(b)?),
        Formula::Trait(query) => Formula::Trait(crate::ptc::TraitQuery {
            interface: query.interface,
            subject: term(&query.subject)?,
            arguments: query
                .arguments
                .iter()
                .map(&mut term)
                .collect::<Option<_>>()?,
        }),
        Formula::Kind(query) => Formula::kind(term(&query.ty)?, query.kind),
        Formula::Not(inner) => lift(inner, fuel, depth + 1)?.negate(),
        Formula::And(items) => Formula::And(
            items
                .iter()
                .map(|v| lift(v, fuel, depth + 1))
                .collect::<Option<_>>()?,
        ),
        Formula::Or(items) => Formula::Or(
            items
                .iter()
                .map(|v| lift(v, fuel, depth + 1))
                .collect::<Option<_>>()?,
        ),
        Formula::True | Formula::False | Formula::IntLe(_) => formula.clone(),
    })
}
fn enter(context: &Context, sorts: Vec<Sort>, fuel: &mut usize) -> Option<Context> {
    let mut scope = context.scope().to_vec();
    scope.push(sorts);
    let mut nested = context.theory_in_scope(scope);
    for assumption in context.assumptions() {
        nested.assume(lift(assumption, fuel, 0)?);
    }
    Some(nested)
}
fn equal(
    context: &Context,
    left: &Term,
    right: &Term,
    budget: Budget,
    fuel: &mut usize,
    depth: usize,
) -> Option<bool> {
    charge(fuel)?;
    if depth > budget.max_depth.min(64) {
        return None;
    }
    if left == right {
        return Some(true);
    }
    // Reserve a disjoint allowance for every PTC query, including failed ones.
    let steps = (*fuel).min(4096);
    *fuel -= steps;
    if matches!(
        context.prove(
            Formula::equal(left.clone(), right.clone()),
            Budget { steps, ..budget }
        ),
        Outcome::Entailed(_)
    ) {
        return Some(true);
    }
    Some(match (left, right) {
        (
            Term::Function {
                parameters: a,
                result: x,
            },
            Term::Function {
                parameters: b,
                result: y,
            },
        ) if a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.sort == b.sort) => {
            let nested = enter(context, a.iter().map(|p| p.sort).collect(), fuel)?;
            for (a, b) in a.iter().zip(b) {
                if !equal(&nested, &a.ty, &b.ty, budget, fuel, depth + 1)? {
                    return Some(false);
                }
            }
            equal(&nested, x, y, budget, fuel, depth + 1)?
        }
        _ => {
            let Some(arguments) = context.congruence_arguments(left, right) else {
                return Some(false);
            };
            for (a, b) in arguments {
                if !equal(context, a, b, budget, fuel, depth + 1)? {
                    return Some(false);
                }
            }
            true
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bound(depth: u32, index: usize) -> Term {
        Term::Bound {
            depth,
            index,
            sort: Sort::Type,
        }
    }
    fn function(result: Term) -> Term {
        Term::Function {
            parameters: vec![nir::Parameter {
                sort: Sort::Type,
                ty: Term::Primitive(nir::Primitive::Type),
            }],
            result: Box::new(result),
        }
    }
    #[test]
    fn nested_signature_equalities_lift_only_outer_bindings() {
        let mut context = Context::with_scope(vec![vec![Sort::Type, Sort::Type]]);
        context.assume(Formula::equal(bound(0, 0), bound(0, 1)));
        let a = function(bound(1, 0));
        let b = function(bound(1, 1));
        assert!(equivalent(&context, [(&a, &b)], Budget::default()));
        let inner = function(bound(0, 0));
        assert!(!equivalent(&context, [(&a, &inner)], Budget::default()));
        let outer = function(function(bound(2, 0)));
        let same = function(function(bound(2, 1)));
        assert!(equivalent(&context, [(&outer, &same)], Budget::default()));
        assert!(!equivalent(
            &context,
            [(&a, &b)],
            Budget {
                steps: 0,
                ..Budget::default()
            }
        ));
        assert!(equivalent(&context, [(&a, &b)], Budget::default()));
        let invalid = function(bound(3, 0));
        assert!(!equivalent(
            &context,
            [(&invalid, &invalid)],
            Budget::default()
        ));
    }
    #[test]
    fn nested_callbacks_use_only_admitted_forward_congruence() {
        let mut context = Context::with_scope(vec![vec![Sort::Type, Sort::Type]]);
        context.assume(Formula::equal(bound(0, 0), bound(0, 1)));
        let head = nir::DeclarationId {
            package: 0,
            local: 77,
        };
        let a = function(bound(1, 0));
        let b = function(bound(1, 1));
        let nominal_a = Term::Nominal {
            declaration: head,
            arguments: vec![a.clone()],
        };
        let nominal_b = Term::Nominal {
            declaration: head,
            arguments: vec![b.clone()],
        };
        assert!(equivalent(
            &context,
            [(&nominal_a, &nominal_b)],
            Budget::default()
        ));
        let normal_a = Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments: vec![a.clone()],
        };
        let normal_b = Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments: vec![b.clone()],
        };
        assert!(!equivalent(
            &context,
            [(&normal_a, &normal_b)],
            Budget::default()
        ));
        context.admit_congruence(head);
        assert!(equivalent(
            &context,
            [(&normal_a, &normal_b)],
            Budget::default()
        ));
        assert!(!equivalent(&context, [(&normal_a, &a)], Budget::default()));
        assert!(!equivalent(
            &context,
            [(&normal_a, &nominal_a)],
            Budget::default()
        ));
        let different = Term::NormalForm {
            head: nir::DeclarationId {
                package: 0,
                local: 78,
            },
            sort: Sort::Type,
            arguments: vec![a],
        };
        assert!(!equivalent(
            &context,
            [(&normal_a, &different)],
            Budget::default()
        ));
    }
}
