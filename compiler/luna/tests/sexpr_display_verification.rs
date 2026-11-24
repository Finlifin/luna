//! End-to-end verification of balanced S-expression `Display` implementations
//! and runtime `pure fn` `K::TypeEqual` support.
use comptime_interpretor::Value;
use comptime_runtime::Error as ArithmeticError;
use elaboration::verification::{
    self, Comparison, Condition, Facts, Interval, Limits, Outcome, PlaceId, Polynomial, Unknown,
    copy, effect_usage, optimization, ownership,
};
use iir::{InstanceKey, effect_values::Delivery};
use nir::{
    DeclarationId, Primitive, Sort, Term,
    contracts::{BindingAlignment, CallableProgram, InputFrameErrorKind, ProgramContract},
    effects::Operation,
    phases::{self, Inspection},
    program::{
        EntrySnapshot, Expr, ExprKind, Function, FunctionDeclaration, FunctionPhase, Literal,
        LocalId, Package, SourceRange,
    },
};
use std::collections::{BTreeMap, BTreeSet};

#[path = "support/execution.rs"]
mod execution_support;

use execution_support::compile;

fn assert_balanced_sexpr(text: &str) {
    assert!(!text.is_empty(), "S-expression output must not be empty");
    let mut depth = 0isize;
    let mut in_string = false;
    let mut escaped = false;
    for ch in text.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                assert!(depth >= 0, "unbalanced closing paren in: {text}");
            }
            _ => {}
        }
    }
    assert!(!in_string, "unterminated string literal in: {text}");
    assert_eq!(depth, 0, "unbalanced parentheses in: {text}");
}

#[test]
fn verification_core_types_render_balanced_sexprs() {
    let place_id = PlaceId { owner: 4, local: 9 };
    assert_balanced_sexpr(&place_id.to_string());
    assert_eq!(place_id.to_string(), "(place-id (owner 4) (local 9))");

    let mut limits = Limits::default();
    assert_balanced_sexpr(&limits.to_string());

    for unknown in [
        Unknown::Arithmetic(ArithmeticError::InvalidLiteral),
        Unknown::Arithmetic(ArithmeticError::BudgetExhausted),
        Unknown::Arithmetic(ArithmeticError::SizeLimit),
        Unknown::Arithmetic(ArithmeticError::DivisionByZero),
        Unknown::Arithmetic(ArithmeticError::OutOfRange),
        Unknown::DegreeLimit,
        Unknown::TermLimit,
        Unknown::InvalidInterval,
        Unknown::InvalidMachineWidth,
        Unknown::VersionExhausted,
        Unknown::InsufficientBounds,
        Unknown::UnreachablePath,
        Unknown::UnrelatedFacts,
        Unknown::ConditionDepthLimit,
    ] {
        let rendered = unknown.to_string();
        assert_balanced_sexpr(&rendered);
        assert!(rendered.starts_with("(verification-unknown "));
    }

    let mut facts = Facts::default();
    let bound = Interval::machine(8, false, &mut limits).unwrap();
    let v = facts.assign(place_id, bound.clone()).unwrap();
    let poly = Polynomial::variable(v, &mut limits).unwrap();
    let cond = Condition::PolynomialCompare {
        left: poly.clone(),
        relation: Comparison::GreaterEqual,
        right: Polynomial::default(),
    };
    let proof = facts.prove_condition(&cond, true, &mut limits).unwrap();
    let outcome_proven = poly.prove_within(&bound, &facts, &mut limits);
    let Outcome::Proven(ref range_proof) = outcome_proven else {
        panic!("expected proven range outcome, got {outcome_proven}");
    };
    let outcome_unknown = Outcome::Unknown(Unknown::InsufficientBounds);

    for rendered in [
        v.to_string(),
        bound.to_string(),
        poly.to_string(),
        cond.to_string(),
        Comparison::LessEqual.to_string(),
        facts.to_string(),
        proof.to_string(),
        range_proof.to_string(),
        outcome_proven.to_string(),
        outcome_unknown.to_string(),
    ] {
        assert!(!rendered.is_empty());
        if rendered.starts_with('(') {
            assert_balanced_sexpr(&rendered);
        }
    }

    let place = ownership::Place {
        root: place_id,
        fields: vec![0, 2],
    };
    let captured_env = ownership::CapturedEnv {
        identity: Term::Runtime(nir::RuntimeId {
            instance: 0,
            local: 1,
        }),
        ty: Term::Pointer(Box::new(Term::Primitive(Primitive::U64))),
        deliveries: vec![],
        delivery_locals: BTreeSet::from([place_id]),
        delivery_invalidated: false,
        target: ownership::CapturedEnvTarget::Invalid {
            reason: "escaped \"env\"\nwith (parens)".into(),
        },
        conditional: true,
    };
    let tree = ownership::CleanupTree {
        place: place.clone(),
        availability: ownership::Availability::Conditional,
        children: Some(vec![]),
    };
    let state = ownership::State::default();

    for rendered in [
        ownership::Shape::Fields(vec![ownership::Shape::Value]).to_string(),
        ownership::Availability::Present.to_string(),
        place.to_string(),
        ownership::Error::NotPresent.to_string(),
        captured_env.to_string(),
        tree.to_string(),
        state.to_string(),
    ] {
        if rendered.starts_with('(') {
            assert_balanced_sexpr(&rendered);
        } else {
            assert!(!rendered.is_empty());
        }
    }
    assert!(
        captured_env
            .to_string()
            .contains("\"escaped \\\"env\\\"\\nwith (parens)\"")
    );
}

#[test]
fn nir_phase_and_contract_types_render_balanced_sexprs() {
    let source = SourceRange { start: 12, end: 34 };
    let declaration = DeclarationId {
        package: 1,
        local: 5,
    };
    let binding = LocalId { owner: 5, index: 0 };
    let bool_ty = Term::Primitive(Primitive::Bool);
    let expr = Expr {
        source,
        ty: bool_ty.clone(),
        kind: ExprKind::Literal(Literal::Bool(true)),
    };
    let function = Function {
        interface: FunctionDeclaration {
            source,
            declaration,
            name: "sample".into(),
            phase: FunctionPhase::Runtime,
            inline: false,
            pure: true,
            is_unsafe: false,
            signature: Term::Function {
                parameters: vec![nir::Parameter {
                    sort: Sort::Runtime,
                    ty: bool_ty.clone(),
                }],
                result: Box::new(bool_ty.clone()),
            },
            parameters: vec![binding],
            env_parameters: vec![],
            type_parameters: vec![],
            defaults: vec![],
            preconditions: vec![expr.clone()],
            postconditions: vec![expr.clone()],
            result_binding: Some(LocalId { owner: 5, index: 2 }),
            result_type_binder: None,
            result_guarantees: vec![],
            entry_snapshots: vec![EntrySnapshot {
                parameter: binding,
                binding: LocalId { owner: 5, index: 1 },
            }],
            requirements: vec![],
            capabilities: vec![],
        },
        body: expr.clone(),
        runtime_deliveries: BTreeMap::new(),
    };
    let package = Package {
        functions: vec![function],
        ..Default::default()
    };

    let inspection = Inspection::new(&package);
    assert_balanced_sexpr(&inspection.to_string());
    let checked = inspection.check_package_with_evidence(&mut 10_000).unwrap();
    assert_balanced_sexpr(&checked.to_string());
    assert_balanced_sexpr(&checked.arithmetic_obligations().to_string());

    let phase_err = phases::Error {
        kind: phases::ErrorKind::Invalid,
        source,
        message: "invalid \"phase\"\n(detail)",
    };
    assert_balanced_sexpr(&phase_err.to_string());
    assert!(
        phase_err
            .to_string()
            .contains("\"invalid \\\"phase\\\"\\n(detail)\"")
    );
    assert_eq!(phases::ErrorKind::Budget.to_string(), "budget");
    assert_eq!(phases::ErrorKind::Depth.to_string(), "depth");
    assert_eq!(phases::ErrorKind::Invalid.to_string(), "invalid");

    let view = CallableProgram::function(&package.functions[0]);
    assert_balanced_sexpr(&view.to_string());
    assert_balanced_sexpr(&view.parts().to_string());
    assert!(view.parts().to_string().starts_with("(program-parts "));

    let contract = ProgramContract {
        preconditions: vec![expr.clone()],
        postconditions: vec![expr],
        result_binding: Some(binding),
        entry_snapshots: vec![],
    };
    assert_balanced_sexpr(&contract.to_string());

    let alignment = BindingAlignment {
        aliases: BTreeMap::from([(binding, binding)]),
        result_binding: Some(binding),
        entry_snapshots: vec![],
    };
    assert_balanced_sexpr(&alignment.to_string());

    assert_eq!(
        InputFrameErrorKind::UnavailableDefaultBinding.to_string(),
        "unavailable-default-binding"
    );
}

#[test]
fn program_verification_report_types_render_balanced_sexprs() {
    let decl = DeclarationId {
        package: 2,
        local: 11,
    };
    let installation = effect_usage::Installation {
        instance: InstanceKey {
            declaration: decl,
            arguments: vec![Term::Primitive(Primitive::U32)].into(),
        },
        source: SourceRange { start: 3, end: 9 },
        operation: Operation {
            declaration: decl,
            arguments: vec![],
        },
        upper: 1,
        delivery: Delivery::Copy,
    };
    let report = effect_usage::Report {
        installations: vec![installation.clone()],
    };
    assert_balanced_sexpr(&installation.to_string());
    assert_balanced_sexpr(&report.to_string());
    assert!(report.to_string().contains(&installation.to_string()));

    let elision = optimization::ElisionReport {
        eliminated: vec![],
        fallback_functions: 2,
    };
    assert_balanced_sexpr(&elision.to_string());

    let pending = copy::Classification::Pending("waiting \"for\" (trait)".into());
    assert_balanced_sexpr(&pending.to_string());

    let _ = verification::PlaceId { owner: 0, local: 0 };
}

#[test]
fn runtime_pure_fn_supports_type_equal_expressions() {
    let package = compile(
        "typealias Type = .Type'builtin;
        typealias u8 = .u8'builtin;
        pure fn is_bool() -> bool where T {
            T == bool
        }
        pure fn pair_matches() -> bool where A, B {
            A == B and is_bool<A>()
        }
        pure fn branch_on_type(value: bool) -> bool where T {
            if T == bool { value } else { not value }
        }
        pure fn literal_type_eq() -> bool {
            bool == bool and not (bool == u8)
        }
        fn run() -> bool {
            is_bool<bool>()
                and not is_bool<u8>()
                and pair_matches<bool, bool>()
                and not pair_matches<bool, u8>()
                and branch_on_type<bool>(true)
                and branch_on_type<u8>(false)
                and literal_type_eq()
        }",
    );
    assert!(
        package
            .functions
            .iter()
            .filter(|f| matches!(
                f.name.as_str(),
                "is_bool" | "pair_matches" | "branch_on_type" | "literal_type_eq"
            ))
            .all(|f| f.pure)
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}
