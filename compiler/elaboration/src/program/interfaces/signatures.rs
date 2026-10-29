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

/// Compare contextual method signatures without treating code as ordinary data.
/// Context correspondence is structural, like code ascription; fixed lexical
/// dependencies and default connections remain separate from type equalities.
pub(super) fn equivalent_code(
    context: &Context,
    expected: &nir::code_interfaces::CodeFunctionSignature,
    actual: &nir::code_interfaces::CodeFunctionSignature,
    budget: Budget,
) -> bool {
    let mut fuel = budget.steps;
    if expected.charge(&mut fuel).is_err()
        || actual.charge(&mut fuel).is_err()
        || expected.inputs.len() != actual.inputs.len()
        || expected.clause_domains.len() != actual.clause_domains.len()
    {
        return false;
    }
    equivalent_code_inputs(
        context,
        &expected.inputs,
        &actual.inputs,
        expected
            .clause_domains
            .iter()
            .zip(&actual.clause_domains)
            .collect(),
        vec![(&expected.output, &actual.output)],
        Budget {
            steps: fuel,
            ..budget
        },
    )
}

/// Data results retain contextual input correspondence and ordinary output Types.
pub(super) fn equivalent_data(
    context: &Context,
    expected: &nir::code_interfaces::DataFunctionSignature,
    actual: &nir::code_interfaces::DataFunctionSignature,
    budget: Budget,
) -> bool {
    let mut fuel = budget.steps;
    if expected.charge(&mut fuel).is_err()
        || actual.charge(&mut fuel).is_err()
        || expected.inputs.len() != actual.inputs.len()
        || expected.clause_domains.len() != actual.clause_domains.len()
    {
        return false;
    }
    let mut pairs = expected
        .clause_domains
        .iter()
        .zip(&actual.clause_domains)
        .collect::<Vec<_>>();
    pairs.push((&expected.output, &actual.output));
    equivalent_code_inputs(
        context,
        &expected.inputs,
        &actual.inputs,
        pairs,
        Vec::new(),
        Budget {
            steps: fuel,
            ..budget
        },
    )
}

fn equivalent_code_inputs<'a>(
    context: &Context,
    expected_inputs: &'a [(nir::program::LocalId, nir::code_interfaces::CodeInput)],
    actual_inputs: &'a [(nir::program::LocalId, nir::code_interfaces::CodeInput)],
    mut pairs: Vec<(&'a Term, &'a Term)>,
    mut interfaces: Vec<(
        &'a nir::code_interfaces::ExpressionCodeInterface,
        &'a nir::code_interfaces::ExpressionCodeInterface,
    )>,
    budget: Budget,
) -> bool {
    use nir::code_interfaces::CodeInput;
    use nir::program::LocalId;
    use std::collections::{BTreeMap, BTreeSet};
    let fuel = budget.steps;
    let mut mapping = BTreeMap::new();
    let mut reverse = BTreeMap::new();
    fn connect(
        mapping: &mut BTreeMap<LocalId, LocalId>,
        reverse: &mut BTreeMap<LocalId, LocalId>,
        from: LocalId,
        to: LocalId,
    ) -> bool {
        if mapping.get(&from).is_some_and(|previous| *previous != to)
            || reverse.get(&to).is_some_and(|previous| *previous != from)
        {
            return false;
        }
        mapping.insert(from, to);
        reverse.insert(to, from);
        true
    }
    for ((left_id, left), (right_id, right)) in expected_inputs.iter().zip(actual_inputs) {
        if mapping.contains_key(left_id)
            || reverse.contains_key(right_id)
            || !connect(&mut mapping, &mut reverse, *left_id, *right_id)
        {
            return false;
        }
        match (left, right) {
            (CodeInput::Data(left), CodeInput::Data(right)) => pairs.push((left, right)),
            (CodeInput::Pattern(left), CodeInput::Pattern(right)) => {
                if !left.outputs.keys().eq(right.outputs.keys()) {
                    return false;
                }
                pairs.push((&left.subject, &right.subject));
                pairs.extend(left.outputs.values().zip(right.outputs.values()));
            }
            (CodeInput::Code(left), CodeInput::Code(right)) => interfaces.push((left, right)),
            _ => return false,
        }
    }
    for (left, right) in &interfaces {
        if left.infer_context
            || right.infer_context
            || left.context.len() != right.context.len()
            || left.ports.len() != right.ports.len()
            || left.defaults.len() != right.defaults.len()
        {
            return false;
        }
        let left_ports: BTreeSet<_> = left.ports.values().copied().collect();
        let right_ports: BTreeSet<_> = right.ports.values().copied().collect();
        if left_ports.len() != left.ports.len()
            || right_ports.len() != right.ports.len()
            || !left_ports.iter().all(|id| left.context.contains_key(id))
            || !right_ports.iter().all(|id| right.context.contains_key(id))
        {
            return false;
        }
        for ((from, left_ty), (to, right_ty)) in left.context.iter().zip(&right.context) {
            let exposed = left_ports.contains(from);
            if exposed != right_ports.contains(to)
                || (!exposed && from != to)
                || !connect(&mut mapping, &mut reverse, *from, *to)
            {
                return false;
            }
            pairs.push((left_ty, right_ty));
        }
        pairs.push((&left.result, &right.result));
    }
    for (left, right) in interfaces {
        for (port, target) in &left.defaults {
            if !left.context.contains_key(port) {
                return false;
            }
            let Some(to) = mapping.get(port) else {
                return false;
            };
            if right.defaults.get(to) != Some(mapping.get(target).unwrap_or(target)) {
                return false;
            }
        }
    }
    // PTC receives type propositions only. Malformed value-shaped payloads
    // must not become candidate type equalities, even when identical.
    if pairs
        .iter()
        .any(|(left, right)| left.sort() != Sort::Type || right.sort() != Sort::Type)
    {
        return false;
    }
    equivalent(
        context,
        pairs,
        Budget {
            steps: fuel,
            ..budget
        },
    )
}

/// Pattern outputs preserve their names and success types; they have no
/// expression context ports or default connections to alpha-rename.
pub(super) fn equivalent_pattern(
    context: &Context,
    expected: &nir::code_interfaces::PatternFunctionSignature,
    actual: &nir::code_interfaces::PatternFunctionSignature,
    budget: Budget,
) -> bool {
    let mut fuel = budget.steps;
    if expected.charge(&mut fuel).is_err()
        || actual.charge(&mut fuel).is_err()
        || expected.inputs.len() != actual.inputs.len()
        || expected.clause_domains.len() != actual.clause_domains.len()
        || !expected
            .output
            .outputs
            .keys()
            .eq(actual.output.outputs.keys())
    {
        return false;
    }
    let mut pairs = expected
        .clause_domains
        .iter()
        .zip(&actual.clause_domains)
        .collect::<Vec<_>>();
    pairs.push((&expected.output.subject, &actual.output.subject));
    pairs.extend(
        expected
            .output
            .outputs
            .values()
            .zip(actual.output.outputs.values()),
    );
    equivalent_code_inputs(
        context,
        &expected.inputs,
        &actual.inputs,
        pairs,
        Vec::new(),
        Budget {
            steps: fuel,
            ..budget
        },
    )
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
    fn code_signature(owner: u32, ty: Term) -> nir::code_interfaces::CodeFunctionSignature {
        use nir::code_interfaces::{CodeFunctionSignature, CodeInput, ExpressionCodeInterface};
        use nir::program::LocalId;
        use std::collections::BTreeMap;
        let id = |index| LocalId { owner, index };
        let input = ExpressionCodeInterface {
            infer_context: false,
            context: BTreeMap::from([(id(2), ty.clone())]),
            ports: BTreeMap::from([(format!("port_{owner}"), id(2))]),
            defaults: BTreeMap::new(),
            result: ty.clone(),
        };
        let output = ExpressionCodeInterface {
            infer_context: false,
            context: BTreeMap::from([(id(3), ty.clone())]),
            ports: BTreeMap::from([(format!("result_{owner}"), id(3))]),
            defaults: BTreeMap::from([(id(3), id(2))]),
            result: ty.clone(),
        };
        CodeFunctionSignature {
            clause_domains: vec![],
            inputs: vec![
                (id(0), CodeInput::Data(ty)),
                (id(1), CodeInput::Code(input)),
            ],
            output,
        }
    }
    fn boolean() -> Term {
        Term::Primitive(nir::Primitive::Bool)
    }
    #[test]
    fn pattern_signatures_preserve_output_names_types_and_input_categories() {
        use nir::code_interfaces::{CodeInput, PatternCodeInterface, PatternFunctionSignature};
        use nir::program::LocalId;
        use std::collections::BTreeMap;
        let output = PatternCodeInterface {
            subject: boolean(),
            outputs: BTreeMap::from([("value".into(), boolean())]),
        };
        let expected = PatternFunctionSignature {
            clause_domains: vec![Term::Primitive(nir::Primitive::Type)],
            inputs: vec![(
                LocalId { owner: 1, index: 0 },
                CodeInput::Pattern(output.clone()),
            )],
            output,
        };
        let context = Context::with_scope(vec![]);
        let mut actual = expected.clone();
        actual.inputs[0].0.owner = 2;
        assert!(equivalent_pattern(
            &context,
            &expected,
            &actual,
            Budget::default()
        ));
        assert!(!equivalent_pattern(
            &context,
            &expected,
            &actual,
            Budget {
                steps: 0,
                ..Budget::default()
            }
        ));
        let mut changed = actual.clone();
        changed.output.outputs = BTreeMap::from([("other".into(), boolean())]);
        assert!(!equivalent_pattern(
            &context,
            &expected,
            &changed,
            Budget::default()
        ));
        let mut changed = actual.clone();
        changed.output.subject = Term::Primitive(nir::Primitive::Char);
        assert!(!equivalent_pattern(
            &context,
            &expected,
            &changed,
            Budget::default()
        ));
        let mut changed = actual.clone();
        changed.inputs[0].1 = CodeInput::Data(boolean());
        assert!(!equivalent_pattern(
            &context,
            &expected,
            &changed,
            Budget::default()
        ));
        let mut changed = actual.clone();
        changed
            .output
            .outputs
            .insert("value".into(), Term::Bool(true));
        assert!(!equivalent_pattern(
            &context,
            &changed,
            &changed,
            Budget::default()
        ));
        let mut changed = actual.clone();
        changed.inputs.push(changed.inputs[0].clone());
        assert!(!equivalent_pattern(
            &context,
            &changed,
            &changed,
            Budget::default()
        ));
    }

    #[test]
    fn data_method_signatures_preserve_code_inputs_and_result_identity() {
        use nir::code_interfaces::{CodeInput, DataFunctionSignature};
        let make = |owner| {
            let signature = code_signature(owner, boolean());
            DataFunctionSignature {
                clause_domains: signature.clause_domains,
                inputs: signature.inputs,
                output: boolean(),
            }
        };
        let expected = make(1);
        let actual = make(2);
        let context = Context::with_scope(vec![]);
        assert!(equivalent_data(
            &context,
            &expected,
            &actual,
            Budget::default()
        ));
        let mut changed = actual.clone();
        changed.output = Term::Primitive(nir::Primitive::Char);
        assert!(!equivalent_data(
            &context,
            &expected,
            &changed,
            Budget::default()
        ));
        changed = actual.clone();
        changed.inputs[1].1 = CodeInput::Data(boolean());
        assert!(!equivalent_data(
            &context,
            &expected,
            &changed,
            Budget::default()
        ));
        assert!(!equivalent_data(
            &context,
            &expected,
            &actual,
            Budget {
                steps: 0,
                ..Budget::default()
            }
        ));
        assert!(equivalent_data(
            &context,
            &expected,
            &actual,
            Budget::default()
        ));
    }

    #[test]
    fn contextual_signatures_preserve_connections_under_binding_renaming() {
        let expected = code_signature(1, boolean());
        let actual = code_signature(2, boolean());
        let context = Context::with_scope(vec![]);
        assert!(equivalent_code(
            &context,
            &expected,
            &actual,
            Budget::default()
        ));
        let mut changed = actual.clone();
        *changed.output.defaults.values_mut().next().unwrap() =
            nir::program::LocalId { owner: 2, index: 0 };
        assert!(!equivalent_code(
            &context,
            &expected,
            &changed,
            Budget::default()
        ));
        changed.output.defaults.clear();
        assert!(!equivalent_code(
            &context,
            &expected,
            &changed,
            Budget::default()
        ));
        assert!(equivalent_code(
            &context,
            &expected,
            &actual,
            Budget::default()
        ));
    }
    #[test]
    fn contextual_signatures_reject_category_and_interface_shape_mismatches() {
        use nir::code_interfaces::CodeInput;
        let expected = code_signature(1, boolean());
        let actual = code_signature(2, boolean());
        let context = Context::with_scope(vec![]);
        let mut cases = Vec::new();
        let mut changed = actual.clone();
        changed.inputs[1].1 = CodeInput::Data(boolean());
        cases.push(changed);
        let mut changed = actual.clone();
        changed.inputs.pop();
        cases.push(changed);
        let mut changed = actual.clone();
        changed.output.infer_context = true;
        cases.push(changed);
        let mut changed = actual.clone();
        changed.inputs[1].0 = changed.inputs[0].0;
        cases.push(changed);
        let mut changed = actual.clone();
        changed.output.ports.insert(
            "missing".into(),
            nir::program::LocalId {
                owner: 2,
                index: 99,
            },
        );
        cases.push(changed);
        let mut changed = actual.clone();
        changed
            .output
            .ports
            .insert("alias".into(), nir::program::LocalId { owner: 2, index: 3 });
        cases.push(changed);
        for changed in cases {
            assert!(
                !equivalent_code(&context, &expected, &changed, Budget::default()),
                "{changed}"
            );
        }
    }
    #[test]
    fn contextual_signatures_do_not_abstract_fixed_dependencies() {
        let mut expected = code_signature(1, boolean());
        expected.output.ports.clear();
        expected.output.defaults.clear();
        let actual = expected.clone();
        let context = Context::with_scope(vec![]);
        assert!(equivalent_code(
            &context,
            &expected,
            &actual,
            Budget::default()
        ));
        let mut exposed = actual.clone();
        exposed.output.ports.insert(
            "new_port".into(),
            nir::program::LocalId { owner: 1, index: 3 },
        );
        assert!(!equivalent_code(
            &context,
            &expected,
            &exposed,
            Budget::default()
        ));
        let mut renamed = code_signature(2, boolean());
        renamed.output.ports.clear();
        renamed.output.defaults.clear();
        assert!(!equivalent_code(
            &context,
            &expected,
            &renamed,
            Budget::default()
        ));
    }
    #[test]
    fn contextual_signatures_preserve_shared_context_binding_identity() {
        use nir::code_interfaces::CodeInput;
        let mut expected = code_signature(1, boolean());
        let CodeInput::Code(input) = &expected.inputs[1].1 else {
            unreachable!()
        };
        expected.output = input.clone();
        let actual = code_signature(2, boolean());
        let context = Context::with_scope(vec![]);
        assert!(!equivalent_code(
            &context,
            &expected,
            &actual,
            Budget::default()
        ));
        let mut aligned = actual;
        let CodeInput::Code(input) = &aligned.inputs[1].1 else {
            unreachable!()
        };
        aligned.output = input.clone();
        assert!(equivalent_code(
            &context,
            &expected,
            &aligned,
            Budget::default()
        ));
    }
    #[test]
    fn contextual_signatures_use_type_premises_without_unfolding_heads() {
        let a = nir::DeclarationId {
            package: 1,
            local: 10,
        };
        let b = nir::DeclarationId {
            package: 1,
            local: 11,
        };
        let left = Term::Rigid {
            declaration: a,
            sort: Sort::Type,
        };
        let right = Term::Rigid {
            declaration: b,
            sort: Sort::Type,
        };
        let expected = code_signature(1, left.clone());
        let actual = code_signature(2, right.clone());
        let mut context = Context::with_scope(vec![]);
        assert!(!equivalent_code(
            &context,
            &expected,
            &actual,
            Budget::default()
        ));
        context.assume(Formula::equal(left.clone(), right));
        assert!(equivalent_code(
            &context,
            &expected,
            &actual,
            Budget::default()
        ));
        let mut headed = actual.clone();
        headed.output.result = Term::NormalForm {
            head: nir::DeclarationId {
                package: 1,
                local: 12,
            },
            sort: Sort::Type,
            arguments: vec![left],
        };
        assert!(!equivalent_code(
            &context,
            &expected,
            &headed,
            Budget::default()
        ));
    }
    #[test]
    fn contextual_signature_budget_failure_does_not_affect_later_comparison() {
        let expected = code_signature(1, boolean());
        let actual = code_signature(2, boolean());
        let context = Context::with_scope(vec![]);
        assert!(!equivalent_code(
            &context,
            &expected,
            &actual,
            Budget {
                steps: 0,
                ..Budget::default()
            }
        ));
        assert!(equivalent_code(
            &context,
            &expected,
            &actual,
            Budget::default()
        ));
        let mut value_shaped = actual.clone();
        value_shaped.output.result = Term::Bool(true);
        assert!(!equivalent_code(
            &context,
            &value_shaped,
            &value_shaped,
            Budget::default()
        ));
        let mut invalid = actual.clone();
        invalid.output.result = bound(0, 0);
        assert!(!equivalent_code(
            &context,
            &invalid,
            &invalid,
            Budget::default()
        ));
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
