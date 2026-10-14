use comptime_interpretor::{Budget, Interpreter, Value};
use nir::normal_forms::{Head, Kind, Parameter as Input};
use nir::program::{Expr, ExprKind as K, Function, FunctionPhase, LocalId, NormalFormArgument, Package, SourceRange};
use nir::{DeclarationId, Parameter, Primitive as P, Sort, Term};

fn id(local: u32) -> DeclarationId { DeclarationId { package: 21, local } }
fn ty(p: P) -> Term { Term::Primitive(p) }
fn list_type() -> Term { Term::NormalForm { head: id(1), sort: Sort::Type, arguments: vec![ty(P::Type)] } }
fn tuple_type(elements: Vec<Term>) -> Term {
    Term::NormalForm { head: id(2), sort: Sort::Type, arguments: vec![Term::TypeListValue(elements)] }
}
fn package() -> Package {
    let binding = LocalId { owner: 3, index: 0 };
    let source = SourceRange { start: 0, end: 1 };
    Package {
        normal_forms: vec![
            Head { role: nir::normal_forms::Role::Opaque, declaration: id(1), requirements: vec![], parameters: vec![Input {
                declaration: id(10), binding: LocalId { owner: 1, index: 0 }, kind: Kind::Type, ty: ty(P::Type),
            }] },
            Head { role: nir::normal_forms::Role::Opaque, declaration: id(2), requirements: vec![], parameters: vec![Input {
                declaration: id(20), binding: LocalId { owner: 2, index: 0 }, kind: Kind::TypeList, ty: list_type(),
            }] },
        ],
        functions: vec![Function {
            declaration: id(3), name: "BuildTuple".into(), phase: FunctionPhase::Comptime, inline: false, pure: true,
            type_parameters: vec![], requirements: vec![], capabilities: vec![],
            preconditions: vec![], postconditions: vec![], result_binding: None, entry_snapshots: vec![],
            parameters: vec![binding], defaults: vec![],
            signature: Term::Function { parameters: vec![Parameter { sort: Sort::ComptimeValue, ty: list_type() }], result: Box::new(ty(P::Type)) },
            body: Expr { ty: ty(P::Type), source, kind: K::NormalFormType {
                head: id(2), plan: None, arguments: vec![NormalFormArgument::Value(Expr {
                    ty: list_type(), source, kind: K::Local(binding),
                })],
            } },
        }],
        ..Default::default()
    }
}

#[test]
fn collected_type_list_crosses_invocation_and_preserves_tuple_head() {
    let package = package();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    for elements in [vec![], vec![ty(P::U8)], vec![ty(P::U8), ty(P::Bool), ty(P::U8)]] {
        let result = vm.invoke(id(3), vec![Value::TypeList(elements.clone())]).unwrap();
        assert_eq!(result, Value::Type(tuple_type(elements.clone())));
        assert_ne!(result, Value::Type(Term::Tuple(elements.clone())));
        let thawed = vm.invoke_with_normal_arguments(id(3),
            vec![(list_type(), Term::TypeListValue(elements.clone()))]).unwrap();
        assert_eq!(thawed, Value::Type(tuple_type(elements)));
    }
}

#[test]
fn type_list_constant_nodes_validate_elements_domain_and_substitution() {
    let mut package=package();
    let source=SourceRange {start:0,end:1};
    package.functions[0].parameters.clear();
    package.functions[0].signature=Term::Function {parameters:vec![],result:Box::new(list_type())};
    for types in [vec![],vec![ty(P::Bool),ty(P::U8)]] {
        package.functions[0].body=Expr {source,ty:list_type(),kind:K::TypeListValue(types.clone())};
        let mut vm=Interpreter::new(&package,Budget::default()).unwrap();
        assert_eq!(vm.invoke(id(3),vec![]).unwrap(),Value::TypeList(types));
    }
    for invalid in [Term::Bool(true),Term::Rigid {declaration:id(99),sort:Sort::Type}] {
        package.functions[0].body.kind=K::TypeListValue(vec![invalid]);
        let mut vm=Interpreter::new(&package,Budget::default()).unwrap();
        assert!(vm.invoke(id(3),vec![]).is_err());
    }
    let mut constant=Expr {source,ty:list_type(),kind:K::TypeListValue(vec![Term::Rigid {declaration:id(99),sort:Sort::Type}])};
    constant.rewrite_types(&mut |t|t.instantiate_types(&[id(99)],&[ty(P::Bool)])).unwrap();
    assert_eq!(constant.kind,K::TypeListValue(vec![ty(P::Bool)]));
    assert!(constant.to_string().contains("(type-list-value"));
    package.functions[0].body=constant;
    let mut vm=Interpreter::new(&package,Budget::default()).unwrap();
    assert_eq!(vm.invoke(id(3),vec![]).unwrap(),Value::TypeList(vec![ty(P::Bool)]));
    package.functions[0].body.ty=ty(P::Type);
    assert!(Interpreter::new(&package,Budget::default()).unwrap().invoke(id(3),vec![]).is_err());
    package.functions[0].body.ty=list_type();
    package.normal_forms.pop();
    assert!(Interpreter::new(&package,Budget::default()).unwrap().invoke(id(3),vec![]).is_err());
}

#[test]
fn list_input_contract_rejects_tuple_wrong_elements_and_recovers() {
    let package = package();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    for bad in [Value::Tuple(vec![Value::Type(ty(P::U8))]), Value::TypeList(vec![Term::Bool(true)]), Value::Type(ty(P::U8))] {
        assert!(vm.invoke(id(3), vec![bad]).is_err());
        assert_eq!(vm.invoke(id(3), vec![Value::TypeList(vec![ty(P::Bool)])]).unwrap(),
            Value::Type(tuple_type(vec![ty(P::Bool)])));
    }
    assert!(vm.invoke_with_normal_arguments(id(3), vec![(list_type(), Term::TupleValue(vec![ty(P::U8)]))]).is_err());
    assert!(vm.validate_immutable_input(&Value::TypeList(vec![ty(P::Bool)]), &list_type()).is_ok());
}

#[test]
fn list_domain_metadata_is_explicit_and_preserves_declared_type_identity() {
    let mut package = package();
    assert!(nir::normal_forms::validate(&package).is_ok());
    assert!(package.normal_forms[1].to_string().contains("(kind type-list)"));
    package.normal_forms[1].parameters[0].kind = Kind::Value;
    assert!(nir::normal_forms::validate(&package).is_err());
    package.normal_forms[1].parameters[0].kind = Kind::TypeList;
    for invalid in [ty(P::Type), ty(P::Bool), Term::Tuple(vec![ty(P::Type)])] {
        package.normal_forms[1].parameters[0].ty = invalid;
        assert!(nir::normal_forms::validate(&package).is_err());
    }
}

#[test]
fn contextual_tuple_types_preserve_the_head_before_and_after_their_context_arrives() {
    use nir::program::{NormalFormInputPlan, TypePredicate};
    let mut package = package();
    package.normal_forms[1].role = nir::normal_forms::Role::Tuple;
    package.normal_forms.push(Head {
        role: nir::normal_forms::Role::Opaque, declaration: id(4), requirements: vec![],
        parameters: vec![
            Input { declaration: id(40), binding: LocalId { owner: 4, index: 0 }, kind: Kind::Type, ty: ty(P::Type) },
            Input { declaration: id(41), binding: LocalId { owner: 4, index: 1 }, kind: Kind::Value,
                ty: Term::Rigid { declaration: id(40), sort: Sort::Type } },
        ],
    });
    let source = SourceRange { start: 0, end: 1 };
    let type_input = |value| NormalFormArgument::Value(Expr { source, ty: ty(P::Type), kind: K::TypeValue(value) });
    package.functions[0].parameters.clear();
    package.functions[0].signature = Term::Function { parameters: vec![], result: Box::new(ty(P::Type)) };
    for order in [vec![0, 1], vec![1, 0]] {
        package.functions[0].body.kind = K::NormalFormType {
            head: id(4),
            plan: Some(NormalFormInputPlan { preparations: vec![], order, bindings: vec![LocalId { owner: 3, index: 1 }, LocalId { owner: 3, index: 2 }] }),
            arguments: vec![type_input(ty(P::Type)), NormalFormArgument::Tuple {
                source, type_head: Some(id(2)), items: vec![type_input(ty(P::Bool)), type_input(ty(P::U8))],
            }],
        };
        assert!(package.functions[0].body.to_string().contains("(type-head"));
        let expected = Value::Type(Term::NormalForm { head: id(4), sort: Sort::Type,
            arguments: vec![ty(P::Type), tuple_type(vec![ty(P::Bool), ty(P::U8)])] });
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(vm.invoke(id(3), vec![]).unwrap(), expected);

        // A contextual Tuple construction must enforce the inner head's
        // formation contract, not just the surrounding normal-form contract.
        let mut denied = package.clone();
        denied.normal_forms[1].requirements.push(TypePredicate::Equal(ty(P::Bool), ty(P::U8)));
        let mut vm = Interpreter::new(&denied, Budget::default()).unwrap();
        assert!(vm.invoke(id(3), vec![]).is_err());
        let mut unbound = package.clone();
        unbound.normal_forms[1].role = nir::normal_forms::Role::Opaque;
        let mut vm = Interpreter::new(&unbound, Budget::default()).unwrap();
        assert!(vm.invoke(id(3), vec![]).is_err());
    }
}

#[test]
fn registered_list_heads_admit_reduced_inputs_and_reject_pending_concatenation() {
    let package = package();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    let parts = vec![Term::TypeListValue(vec![ty(P::Bool)]),Term::TypeListValue(vec![ty(P::U8)])];
    let pending = Term::NormalForm {head:id(2),sort:Sort::Type,arguments:vec![Term::TypeListConcat(parts.clone())]};
    assert!(vm.validate_immutable_input(&Value::Type(pending), &ty(P::Type)).is_err());
    let completed = Term::NormalForm {head:id(2),sort:Sort::Type,arguments:vec![Term::type_list_concat(parts)]};
    assert!(vm.validate_immutable_input(&Value::Type(completed.clone()), &ty(P::Type)).is_ok());
    assert_eq!(completed, tuple_type(vec![ty(P::Bool),ty(P::U8)]));
}
