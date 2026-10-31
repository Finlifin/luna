//! Parentheses construct values; Tuple is an explicit normal-form type head.
#[path = "support/frozen_lists.rs"]
mod frozen_lists;
#[path = "support/execution.rs"]
mod support;
use comptime_interpretor::Value;
use nir::{Primitive, Term};

#[test]
fn tuple_examples_execute_with_explicit_type_constructor_bindings() {
    for (source, expected) in [
        (
            include_str!("../../../development/examples/computed-tuple-types.fl"),
            Value::Tuple(vec![
                Value::Tuple(vec![
                    Value::Tuple(vec![Value::Bool(true), Value::Bool(false)]),
                    Value::Bool(true),
                ]),
                Value::Tuple(vec![
                    Value::Bool(false),
                    Value::Tuple(vec![Value::Char('z'), Value::Bool(true)]),
                ]),
            ]),
        ),
        (
            include_str!("../../../development/examples/generic-calls.fl"),
            Value::Tuple(vec![Value::Bool(true), Value::Char('x')]),
        ),
        (
            include_str!("../../../development/examples/type-requirements.fl"),
            Value::Tuple(vec![Value::Bool(true), Value::Char('x')]),
        ),
    ] {
        let package = support::compile_with_source_bindings(
            source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("input.TupleElements"),
                tuple: Some("input.Tuple"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            expected
        );
    }
}

fn compile(body: &str, bound: bool) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        &format!(
            "typealias Type=.Type'builtin;
            pure comptime fn List<T>->Type {{T}}
            pure comptime fn Tuple<...Es:List<Type>>->Type {{bool}}
            {body}"
        ),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.List"),
            tuple: bound.then_some("input.Tuple"),
            ..Default::default()
        },
    )
}

#[test]
fn parentheses_do_not_construct_types_in_any_type_context() {
    for bound in [false, true] {
        for source in [
            "fn bad(value:(bool,bool)) {}",
            "fn bad(value:(bool,)) {}",
            "fn bad(value:((bool,bool))) {}",
            "fn bad()->(bool,bool) {(true,false)}",
            "typealias Bad=(bool,bool);",
            "typealias Bad=();",
            "typealias Bad=(bool,);",
            "fn bad(value:()) {}",
            "struct Bad {field:(bool,bool)}",
            "struct Bad {field:*(bool,bool)}",
            "typealias Bad=*(bool,);",
            "trait Bad {assoc Element:Type=(bool,bool);}",
            "comptime fn Bad()->Type {(bool,bool)}",
            "comptime fn Bad(T:Type)->Type {(T,bool)}",
            "comptime fn Bad()->Type {()}",
            "comptime fn Bad(T:Type)->Type {if true {(T,bool)} else {T}}",
            "pure comptime fn Identity<T>->Type {T} typealias Bad=Identity<(bool,bool)>;",
            "typealias Bad=Tuple<(bool,bool)>;",
            "typealias Bad=Tuple<()>;",
            "typealias Bad=Tuple<bool,(bool,bool)>;",
            "typealias Bad=Tuple<(bool,)>;",
            "fn bad(value:fn()->(bool,bool)) {}",
            "fn bad() {let value:Type=(bool,bool);}",
            "typealias Bad=Tuple<Tuple<bool>,(bool,)>;",
            "fn bad(value:Tuple<bool,*(bool,bool)>) {}",
            "fn bad(value:fn(bool)->(bool,)) {}",
            "fn bad(value:fn(bool)->()) {}",
        ] {
            let error = compile(source, bound).unwrap_err();
            assert!(error.contains("value, not a type"), "{source}: {error}");
        }
    }
}

#[test]
fn tuple_values_cannot_become_types_through_bindings_or_calls() {
    for source in [
        "comptime fn Bad()->Type {let types=(bool,bool); types}",
        "comptime fn Bad()->Type {let types=(bool,); types}",
        "comptime fn Bad()->Type {let types=(); types}",
        "comptime fn Packet()->Tuple<Type,Type> {(bool,bool)}
         comptime fn Bad()->Type {Packet()}",
        "comptime fn Packet()->Tuple<Type> {(bool,)}
         comptime fn Bad()->Type {Packet()}",
        "comptime fn Packet()->Tuple<> {()}
         comptime fn Bad()->Type {Packet()}",
    ] {
        assert!(
            compile(source, true).is_err(),
            "accepted tuple as Type: {source}"
        );
    }

    // Extracting an element of a tuple of Type values still yields a Type.
    let package = compile(
        "comptime fn Selected()->Type {let (first,second)=(bool,.u8'builtin); first}",
        true,
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "Selected", 100_000).unwrap(),
        Value::Type(Term::Primitive(Primitive::Bool))
    );
}

#[test]
fn grouping_and_tuple_values_remain_distinct_from_type_construction() {
    let package = compile(
        "typealias Flag=((bool));
         typealias PairType=(Tuple<bool,bool>);
         comptime fn Packet()->Tuple<Type,Type> {(bool,.u8'builtin)}
         comptime fn Pair(T:Type)->Type {Tuple<T,bool>}
         comptime fn Selected()->Type {Pair(.u8'builtin)}
         comptime fn Single()->Tuple<Type> {(bool,)}
         comptime fn Empty()->Tuple<> {()}
         fn run()->Flag {
             let pair:PairType = (true,false);
             let (a,b)=pair;
             let single:Tuple<bool> = (a,);
             let (value,)=single;
             let empty:Tuple<> = ();
             value and not b
         }",
        true,
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::invocation::invoke(&package, "Single", 100_000).unwrap(),
        Value::Tuple(vec![Value::Type(Term::Primitive(Primitive::Bool))])
    );
    assert_eq!(
        luna::invocation::invoke(&package, "Empty", 100_000).unwrap(),
        Value::Tuple(vec![])
    );
    assert_eq!(
        luna::invocation::invoke(&package, "Packet", 100_000).unwrap(),
        Value::Tuple(vec![
            Value::Type(Term::Primitive(Primitive::Bool)),
            Value::Type(Term::Primitive(Primitive::U8))
        ])
    );
    let Value::Type(Term::NormalForm {
        head, arguments, ..
    }) = luna::invocation::invoke(&package, "Selected", 100_000).unwrap()
    else {
        panic!("Tuple must retain its normal-form head")
    };
    assert_eq!(head, support::id(&package, "Tuple"));
    assert_eq!(
        arguments,
        vec![frozen_lists::types(
            &package,
            vec![
                Term::Primitive(Primitive::U8),
                Term::Primitive(Primitive::Bool)
            ]
        )]
    );
}

#[test]
fn empty_tuple_patterns_use_layout_and_remain_exhaustive() {
    let package = compile(
        "fn inspect(packet:Tuple<>)->bool {packet match {()=>true}}
         comptime fn Empty()->Tuple<> {()}
         comptime fn Check()->bool {let ()=Empty();true}
         fn run()->bool {
             let ()=();
             let ((),((),flag)):Tuple<Tuple<>,Tuple<Tuple<>,bool>> = ((),((),true));
             inspect(()) and flag
         }",
        true,
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::invocation::invoke(&package, "Check", 100_000).unwrap(),
        Value::Bool(true)
    );
    for source in [
        "fn bad(packet:Tuple<bool>) {let ()=packet;}",
        "fn bad(packet:bool) {let ()=packet;}",
        "fn bad(packet:Tuple<>) {let (value,)=packet;}",
    ] {
        assert!(compile(source, true).is_err(), "{source}");
    }
}

#[test]
fn empty_tuple_type_patterns_observe_the_tuple_view() {
    let package = compile(
        "typealias Bool=bool;
         pure comptime fn Opaque<T>->Type {Tuple<>}
         comptime fn is_empty(value:Type)->bool {
             value match {()=>true,_=>false}
         }
         comptime fn unresolved()->bool {is_empty(Opaque<bool>)}
         comptime fn nested(value:Type)->bool {
             value match {((), 'Bool)=>true,_=>false}
         }
         comptime fn Check()->bool {
             is_empty(Tuple<>) and not is_empty(Tuple<bool>)
                 and not is_empty(bool)
                 and nested(Tuple<Tuple<>,bool>)
                 and not nested(Tuple<Tuple<bool>,bool>)
         }",
        true,
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "Check", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert!(
        luna::invocation::invoke(&package, "unresolved", 100_000)
            .unwrap_err()
            .contains("TypeContractUnresolved")
    );
}

#[test]
fn empty_type_views_preserve_candidate_bindings_and_coverage() {
    let package = compile(
        "typealias Bool=bool;
         typealias Char=.char'builtin;
         comptime fn Pick(value:Type)->Type {
             value match {
                 (Item, ()) or ((), Item)=>Item,
                 _=>Bool,
             }
         }
         comptime fn PickGuarded(value:Type)->Type {
             value match {
                 (Item, ()) if Item == Bool=>Item,
                 ((), Item)=>Item,
                 _=>Bool,
             }
         }
         comptime fn Check()->bool {
             Pick(Tuple<Char,Tuple<>>) == Char
                 and Pick(Tuple<Tuple<>,Char>) == Char
                 and Pick(Tuple<Char,Char>) == Bool
                 and PickGuarded(Tuple<Char,Tuple<>>) == Bool
                 and PickGuarded(Tuple<Tuple<>,Char>) == Char
         }",
        true,
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "Check", 100_000).unwrap(),
        Value::Bool(true)
    );

    // A tuple view describes only one family of types. Even its empty
    // pattern cannot make a match over all Type values exhaustive.
    let error = compile(
        "comptime fn bad(value:Type)->bool {value match {()=>true}}",
        true,
    )
    .unwrap_err();
    assert!(error.contains("exhaustive"), "{error}");
}

#[test]
fn documented_inline_type_pattern_uses_explicit_tuple_identity() {
    let document = include_str!("../../../docs/comptime/inlining-comptime-code.md");
    let source = document
        .split("```flurry\n")
        .skip(1)
        .map(|section| section.split("```").next().unwrap())
        .find(|source| source.contains("fn has_expected_elements()"))
        .expect("the documented inline tuple pattern example must exist");
    let package = compile(source, true).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "has_expected_elements", 100_000).unwrap(),
        Value::Bool(true)
    );
}
