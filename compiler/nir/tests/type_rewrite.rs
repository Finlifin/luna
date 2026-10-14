use nir::program::{Capture, Expr, ExprKind as K, LocalId, Pattern, SourceRange};
use nir::{DeclarationId, Primitive, Sort, Term};
use std::sync::Arc;
fn id(n: u32) -> DeclarationId {
    DeclarationId {
        package: 1,
        local: n,
    }
}
fn rigid() -> Term {
    Term::Rigid {
        declaration: id(1),
        sort: Sort::Type,
    }
}
fn expr(kind: K) -> Expr {
    Expr {
        ty: rigid(),
        source: SourceRange { start: 1, end: 2 },
        kind,
    }
}
#[test]
fn rewrite_visits_hidden_call_identity_and_pattern_expressions() {
    let method = expr(K::TraitMethod {
        interface: id(2),
        member: id(3),
        subject: rigid(),
        interface_arguments: vec![rigid()],
        method_arguments: vec![rigid()],
    });
    let mut pattern = Pattern::Then {
        first: Box::new(Pattern::Wild),
        value: Box::new(method),
        next: Box::new(Pattern::Guard {
            pattern: Box::new(Pattern::Wild),
            condition: Box::new(expr(K::TypeValue(rigid()))),
        }),
    };
    let mut visits = 0;
    pattern
        .rewrite_types(&mut |ty| {
            visits += 1;
            assert_eq!(*ty, rigid());
            Ok::<_, ()>(Term::Primitive(Primitive::Bool))
        })
        .unwrap();
    assert_eq!(visits, 6);
    let dump = pattern.to_string();
    assert!(!dump.contains("(rigid"));
    assert!(dump.contains("trait-method"));
}
#[test]
fn closure_rewrite_preserves_shared_template_and_capture_identity() {
    let binding = LocalId { owner: 5, index: 0 };
    let body = Arc::new(expr(K::Local(binding)));
    let mut closure = expr(K::Closure {
        captures: vec![Capture {
            binding,
            ty: rigid(),
            value: Some(Box::new(expr(K::TypeValue(rigid())))),
        }],
        body: body.clone(),
        parameters: vec![],
    });
    let original = closure.clone();
    let mut visits = 0;
    closure
        .rewrite_types(&mut |_| {
            visits += 1;
            Ok::<_, ()>(Term::Primitive(Primitive::Char))
        })
        .unwrap();
    assert_eq!(visits, 5);
    assert_eq!(body.ty, rigid());
    assert_ne!(closure, original);
    let K::Closure { captures, body, .. } = closure.kind else {
        panic!()
    };
    assert_eq!(captures[0].binding, binding);
    assert!(matches!(body.kind,K::Local(id) if id==binding));
}

#[test]
fn symbolic_type_lists_concatenate_after_substitution_without_changing_heads() {
    use nir::{DeclarationId,DeclarationParameter,Primitive,Sort,Term};
    let input=DeclarationId {package:91,local:1};
    let head=DeclarationId {package:91,local:2};
    let boolean=Term::Primitive(Primitive::Bool);
    let byte=Term::Primitive(Primitive::U8);
    let list=Term::type_list_concat(vec![Term::TypeListValue(vec![boolean.clone()]),
        Term::Rigid {declaration:input,sort:Sort::ComptimeValue},Term::TypeListValue(vec![byte.clone()])]);
    list.check_scope(&[]).unwrap();
    assert!(list.to_string().contains("type-list-concat"));
    let template=Term::NormalForm {head,sort:Sort::Type,arguments:vec![list]};
    for middle in [vec![],vec![byte.clone(),boolean.clone()]] {
        let actual=template.instantiate_parameters(&[DeclarationParameter {declaration:input,sort:Sort::ComptimeValue}],
            &[Term::TypeListValue(middle.clone())]).unwrap();
        let mut elements=vec![boolean.clone()];elements.extend(middle);elements.push(byte.clone());
        assert_eq!(actual,Term::NormalForm {head,sort:Sort::Type,arguments:vec![Term::TypeListValue(elements)]});
    }
    assert!(template.instantiate_parameters(&[DeclarationParameter {declaration:input,sort:Sort::ComptimeValue}],&[Term::Bool(true)]).is_err());
    assert_eq!(Term::type_list_concat(vec![]),Term::TypeListValue(vec![]));
}

#[test]
fn type_list_index_rewriting_reaches_both_operands_and_keeps_head_identity() {
    let list_head=id(40);
    let tuple_head=id(41);
    let list_type=Term::NormalForm {head:list_head,sort:Sort::Type,
        arguments:vec![Term::Primitive(Primitive::Type)]};
    let nested=Term::NormalForm {head:tuple_head,sort:Sort::Type,
        arguments:vec![Term::TypeListValue(vec![rigid()])]};
    let mut index=Expr {
        ty:Term::Primitive(Primitive::Type),source:SourceRange {start:10,end:20},
        kind:K::TypeListIndex(Box::new(Expr {
            ty:list_type.clone(),source:SourceRange {start:11,end:12},
            kind:K::TypeListValue(vec![nested]),
        }),Box::new(Expr {
            ty:Term::Primitive(Primitive::Integer),source:SourceRange {start:13,end:14},
            kind:K::Local(LocalId {owner:2,index:0}),
        })),
    };
    index=index.instantiate_types(&[id(1)],&[Term::Primitive(Primitive::U8)]).unwrap();
    let mut sources=Vec::new();
    nir::visit::expression(&index,&mut 100,|expr,_| {sources.push(expr.source.start);Ok(())}).unwrap();
    sources.sort();assert_eq!(sources,[10,11,13]);
    nir::visit::expression_mut(&mut index,&mut 100,|expr,_| {
        if matches!(expr.kind,K::Local(_)) {expr.kind=K::Literal(nir::program::Literal::Integer("0".into()));}
        Ok(())
    }).unwrap();
    let K::TypeListIndex(list,position)=&index.kind else {panic!("index")};
    assert_eq!(list.ty,list_type);
    assert_eq!(list.kind,K::TypeListValue(vec![Term::NormalForm {head:tuple_head,sort:Sort::Type,
        arguments:vec![Term::TypeListValue(vec![Term::Primitive(Primitive::U8)])]}]));
    assert!(matches!(position.kind,K::Literal(_)));
    assert!(index.to_string().contains("(type-list-index (values"));
}
