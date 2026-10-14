use nir::program::{
    CallArgument, Expr, ExprKind as K, Function, FunctionPhase as Phase, Literal, LocalId, Package,
    SourceRange,
};
use nir::{DeclarationId, Parameter, Primitive, Sort, Term};

fn flag(kind: K) -> Expr {
    Expr {
        kind,
        ty: Term::Primitive(Primitive::Bool),
        source: SourceRange { start: 1, end: 2 },
    }
}
fn function(local: u32, phase: Phase, inputs: &[Sort]) -> Function {
    Function {
        declaration: DeclarationId { package: 1, local },
        name: format!("function{local}"),
        phase,
        inline: false, pure: false,
        signature: Term::Function {
            parameters: inputs
                .iter()
                .map(|sort| Parameter {
                    sort: *sort,
                    ty: Term::Primitive(Primitive::Bool),
                })
                .collect(),
            result: Box::new(Term::Primitive(Primitive::Bool)),
        },
        parameters: inputs
            .iter()
            .enumerate()
            .map(|(index, _)| LocalId {
                owner: local,
                index: index as u32,
            })
            .collect(),
        body: flag(K::Literal(Literal::Bool(true))),
        type_parameters: vec![],
        defaults: vec![],
        preconditions: vec![],
        postconditions: vec![],
        result_binding: None,
        entry_snapshots: vec![],
        capabilities: vec![],
        requirements: vec![],
    }
}
fn call(target: &Function, arguments: Vec<(usize, Expr)>) -> Expr {
    flag(K::Call {
        callee: Box::new(Expr {
            kind: K::Function(target.declaration),
            ty: target
                .signature
                .clone()
                .with_comptime_phase(target.phase == Phase::Comptime),
            source: target.body.source,
        }),
        arguments: arguments
            .into_iter()
            .map(|(parameter, value)| CallArgument {
                parameter,
                value,
                is_default: false,
            })
            .collect(),
        parameter_bindings: target.parameters.clone(),
    })
}

#[test]
fn staged_argument_phase_follows_parameter_slot_not_source_order() {
    let compute = function(1, Phase::Comptime, &[]);
    let runtime = function(2, Phase::Runtime, &[]);
    let target = function(3, Phase::Runtime, &[Sort::Runtime, Sort::ComptimeValue]);
    let mut root = function(4, Phase::Runtime, &[]);
    root.body = call(
        &target,
        vec![(1, call(&compute, vec![])), (0, call(&runtime, vec![]))],
    );
    let mut package = Package {
        functions: vec![compute.clone(), runtime.clone(), target.clone(), root],
        ..Default::default()
    };
    nir::phases::validate(&package).unwrap();
    // Reversing which slots get the calls violates both execution boundaries.
    package.functions[3].body = call(
        &target,
        vec![(1, call(&runtime, vec![])), (0, call(&compute, vec![]))],
    );
    assert!(nir::phases::validate(&package).is_err());
    package.functions[3].body = call(
        &target,
        vec![
            (1, call(&runtime, vec![])),
            (0, flag(K::Literal(Literal::Bool(true)))),
        ],
    );
    assert!(
        nir::phases::validate(&package)
            .unwrap_err()
            .message
            .contains("runtime function is unavailable")
    );
    package.functions[3].body = call(
        &target,
        vec![(1, call(&compute, vec![])), (0, call(&compute, vec![]))],
    );
    assert!(nir::phases::validate(&package).is_err());
}

#[test]
fn staged_context_propagates_through_blocks_and_nested_calls() {
    let compute = function(1, Phase::Comptime, &[]);
    let combine = function(2, Phase::Comptime, &[Sort::Runtime]);
    let target = function(3, Phase::Runtime, &[Sort::ComptimeValue]);
    let mut root = function(4, Phase::Runtime, &[]);
    let nested = flag(K::Block {
        statements: vec![],
        result: Some(Box::new(call(&combine, vec![(0, call(&compute, vec![]))]))),
    });
    root.body = call(&target, vec![(0, nested)]);
    let mut package = Package {
        functions: vec![compute, combine, target.clone(), root],
        ..Default::default()
    };
    nir::phases::validate(&package).unwrap();
    package.functions[3].body = call(&target, vec![(7, flag(K::Literal(Literal::Bool(true))))]);
    assert!(
        nir::phases::validate(&package)
            .unwrap_err()
            .message
            .contains("index out of range")
    );
}

#[test]
fn defaults_are_checked_in_their_parameter_phase_without_a_call_site() {
    let compute = function(1, Phase::Comptime, &[]);
    let runtime = function(2, Phase::Runtime, &[]);
    let mut target = function(3, Phase::Runtime, &[Sort::ComptimeValue, Sort::Runtime]);
    target.defaults = vec![(0, call(&compute, vec![])), (1, call(&runtime, vec![]))];
    let mut package = Package {
        functions: vec![compute.clone(), runtime.clone(), target],
        ..Default::default()
    };
    nir::phases::validate(&package).unwrap();
    package.functions[2].defaults[0].1 = call(&runtime, vec![]);
    assert!(
        nir::phases::validate(&package)
            .unwrap_err()
            .message
            .contains("runtime function is unavailable")
    );
    package.functions[2].defaults[0].1 = call(&compute, vec![]);
    package.functions[2].defaults[1].1 = call(&compute, vec![]);
    assert!(nir::phases::validate(&package).is_err());
}

#[test]
fn nominal_callback_storage_preserves_ordinary_input_frames() {
    let nominal=DeclarationId {package:1,local:50};
    let parameter=DeclarationId {package:1,local:51};
    let definition=nir::program::Struct {declaration:nominal,name:"Box".into(),
        type_parameters:vec![parameter],requirements:vec![],capabilities:vec![],
        fields:vec![("value".into(),Term::Rigid {declaration:parameter,sort:Sort::Type})]};
    let mut root=function(1,Phase::Runtime,&[Sort::Type,Sort::Runtime]);
    let callback=Term::Function {parameters:vec![Parameter {sort:Sort::Runtime,
        ty:Term::Bound {depth:1,index:0,sort:Sort::Type}}],
        result:Box::new(Term::Bound {depth:1,index:0,sort:Sort::Type})};
    let Term::Function {parameters,..}=&mut root.signature else {panic!()};
    parameters[0].ty=Term::Primitive(Primitive::Type);
    parameters[1].ty=Term::Nominal {declaration:nominal,arguments:vec![callback.clone()]};
    let mut package=Package {structs:vec![definition],functions:vec![root],..Default::default()};
    nir::phases::validate(&package).unwrap();
    let Term::Function {parameters,..}=&mut package.functions[0].signature else {panic!()};
    parameters[1].ty=Term::Nominal {declaration:nominal,arguments:vec![Term::ComptimeFunction(Box::new(callback))]};
    assert!(nir::phases::validate(&package).unwrap_err().message.contains("comptime-only"));
    let Term::Function {parameters,..}=&mut package.functions[0].signature else {panic!()};
    parameters[1].ty=Term::Nominal {declaration:nominal,arguments:vec![Term::Bound {depth:3,index:0,sort:Sort::Type}]};
    assert!(nir::phases::validate(&package).is_err());
}

#[test]
fn recursive_runtime_callback_storage_terminates_across_function_frames() {
    let nominal=DeclarationId {package:1,local:80};
    let node=Term::Nominal {declaration:nominal,arguments:vec![]};
    let callback=Term::Function {parameters:vec![],result:Box::new(node.clone())};
    let definition=nir::program::Struct {declaration:nominal,name:"Node".into(),
        type_parameters:vec![],requirements:vec![],capabilities:vec![],fields:vec![("next".into(),callback)]};
    let mut root=function(1,Phase::Runtime,&[Sort::Runtime]);
    let Term::Function {parameters,..}=&mut root.signature else {panic!()};
    parameters[0].ty=node;
    let mut package=Package {structs:vec![definition],functions:vec![root],..Default::default()};
    nir::phases::validate(&package).unwrap();
    package.structs[0].fields.push(("forbidden".into(),Term::Primitive(Primitive::Type)));
    assert!(nir::phases::validate(&package).unwrap_err().message.contains("comptime-only"));
}

#[test]
fn standalone_runtime_domains_preserve_stage_and_scope_checks() {
    let package = Package::default();
    let check = |ty: &Term, scope: &[Vec<Sort>]| nir::phases::validate_runtime_storage(ty, &package, scope);
    let input = Term::Bound { depth: 0, index: 0, sort: Sort::Type };
    assert!(check(&input, &[vec![Sort::Type]]).is_ok());
    assert!(check(&input, &[]).is_err());
    assert!(check(&Term::Bool(true), &[]).is_err());
    assert!(check(&Term::Primitive(Primitive::Type), &[]).is_err());
    let callable = |sort| Term::Function {
        parameters: vec![Parameter {sort, ty: Term::Primitive(Primitive::Type)}],
        result: Box::new(Term::Primitive(Primitive::Bool)),
    };
    assert!(check(&callable(Sort::Type), &[]).is_ok());
    assert!(check(&callable(Sort::Runtime), &[]).is_err());
}

#[test]
fn immutable_type_observations_do_not_authorize_runtime_type_storage() {
    let type_value = Expr {
        source: SourceRange {start: 1,end: 2},
        ty: Term::Primitive(Primitive::Type),
        kind: K::TypeValue(Term::Primitive(Primitive::Bool)),
    };
    let mut root = function(1, Phase::Runtime, &[]);
    root.body = flag(K::TypeEqual(Box::new(type_value.clone()), Box::new(type_value.clone())));
    let mut package = Package {functions:vec![root], ..Default::default()};
    nir::phases::validate(&package).unwrap();
    package.functions[0].body = type_value.clone();
    assert!(nir::phases::validate(&package).is_err());
    let indirect = Expr {kind: K::Block {statements:vec![],result:Some(Box::new(type_value.clone()))}, ..type_value.clone()};
    package.functions[0].body = flag(K::TypeEqual(Box::new(indirect),Box::new(type_value)));
    assert!(nir::phases::validate(&package).is_err());
}

#[test]
fn retained_quote_templates_are_checked_without_becoming_runtime_entries() {
    let inner=function(70,Phase::Runtime,&[]);
    let mut outer=function(71,Phase::Runtime,&[]);
    outer.body=call(&inner,vec![]);
    let mut package=Package {quote_templates:vec![inner.clone(),outer.clone()],..Default::default()};
    nir::phases::validate(&package).unwrap();
    let dump=package.to_string();
    assert_eq!(dump.matches("(quote-template ").count(),2);
    assert!(dump.contains("function70"));
    let mut root=function(72,Phase::Runtime,&[]);
    root.body=call(&outer,vec![]);
    package.functions.push(root);
    assert!(nir::phases::validate(&package).unwrap_err().message.contains("requires deterministic expansion"));
    package.functions.clear();
    let K::Call {callee,..}=&outer.body.kind else {unreachable!()};
    package.quote_templates[1].body=(**callee).clone();
    assert!(nir::phases::validate(&package).unwrap_err().message.contains("cannot escape"));
}

#[test]
fn retained_quote_templates_reject_identity_collisions_and_wrong_phase() {
    let template=function(73,Phase::Runtime,&[]);
    let mut package=Package {quote_templates:vec![template.clone()],..Default::default()};
    package.functions.push(template.clone());
    assert!(nir::phases::validate(&package).unwrap_err().message.contains("quote template identity"));
    package.functions.clear();
    package.quote_templates.push(template.clone());
    assert!(nir::phases::validate(&package).unwrap_err().message.contains("quote template identity"));
    package.quote_templates.pop();
    package.runtime_function_identities.push(nir::program::RuntimeFunctionIdentity {
        declaration:template.declaration,type_parameters:vec![],signature:template.signature,
    });
    assert!(nir::phases::validate(&package).unwrap_err().message.contains("quote template identity"));
    package.runtime_function_identities.clear();
    package.quote_templates[0].phase=Phase::Comptime;
    assert!(nir::phases::validate(&package).unwrap_err().message.contains("quote template identity"));
}

#[test]
fn retained_quote_templates_do_not_hide_invalid_runtime_storage() {
    let mut template=function(74,Phase::Runtime,&[]);
    template.body=Expr {source:SourceRange {start:1,end:2},ty:Term::Primitive(Primitive::Type),
        kind:K::TypeValue(Term::Primitive(Primitive::Bool))};
    let package=Package {quote_templates:vec![template],..Default::default()};
    assert!(nir::phases::validate(&package).is_err());
}
