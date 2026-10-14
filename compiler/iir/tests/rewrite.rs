use nir::{Primitive,Term};
use nir::program::{Expr,ExprKind as K,Literal,NormalFormArgument as A,NormalMatchArm,Pattern,SourceRange};
fn boolean() -> Expr { Expr {source:SourceRange {start:0,end:1},ty:Term::Primitive(Primitive::Bool),kind:K::Literal(Literal::Bool(true))} }

#[test]
fn normal_packet_preparations_participate_in_visiting_and_type_substitution() {
    use nir::program::{NormalFormInputPlan,NormalFormPreparation,LocalId};
    let parameter=nir::DeclarationId {package:1,local:1};
    let generic=Term::Rigid {declaration:parameter,sort:nir::Sort::Type};
    let binding=LocalId {owner:1,index:2};
    let packet=Expr {ty:Term::Tuple(vec![generic.clone()]),kind:K::Tuple(vec![Expr {
        ty:generic.clone(),..boolean()
    }]),..boolean()};
    let mut expression=Expr {kind:K::NormalFormType {
        head:parameter,arguments:vec![A::Value(Expr {ty:generic,kind:K::Local(binding),..boolean()})],
        plan:Some(NormalFormInputPlan {order:vec![0],bindings:vec![LocalId {owner:1,index:3}],
            preparations:vec![NormalFormPreparation {before:0,expression:Expr {
                kind:K::LetPattern {pattern:Pattern::Tuple(vec![Pattern::Binding {binding,inner:Box::new(Pattern::Wild)}]),
                    value:Box::new(packet),guard:None,otherwise:None},ty:Term::Primitive(Primitive::Void),..boolean()
            }}],
        }),
    },ty:Term::Primitive(Primitive::Type),..boolean()};
    let mut literals=0;
    iir::visit::expression_mut(&mut expression,&mut 100,|value,_| {
        if let K::Literal(Literal::Bool(flag))=&mut value.kind {*flag=false;literals+=1;}
        Ok(())
    }).unwrap();
    assert_eq!(literals,1);
    let specialized=expression.instantiate_types(&[parameter],&[Term::Primitive(Primitive::Bool)]).unwrap();
    let mut inspected=0;
    iir::visit::expression(&specialized,&mut 100,|value,_| {
        assert!(!value.ty.mentions_rigid(parameter));
        if matches!(value.kind,K::Literal(Literal::Bool(false))) {inspected+=1;}
        Ok(())
    }).unwrap();
    assert_eq!(inspected,1);
    assert!(iir::visit::expression(&specialized,&mut 2,|_,_|Ok(())).is_err());
}

#[test]
fn expression_rewriting_visits_embedded_patterns_and_preserves_shared_closure_templates() {
    let body=std::sync::Arc::new(Expr {kind:K::NormalFormType {
        plan:None,
        head:nir::DeclarationId {package:0,local:1},
        arguments:vec![A::Match {source:boolean().source,value:boolean(),arms:vec![NormalMatchArm {
            pattern:Pattern::Then {first:Box::new(Pattern::Wild),value:Box::new(boolean()),next:Box::new(Pattern::Wild)},
            guard:Some(boolean()),body:A::Value(boolean()),
        }]}],
    },..boolean()});
    let original=body.clone();
    let mut expression=Expr {kind:K::Closure {parameters:vec![],captures:vec![],body},..boolean()};
    let mut changed=0;
    iir::visit::expression_mut(&mut expression,&mut 100,|expr,_| {
        if let K::Literal(Literal::Bool(value))=&mut expr.kind {*value=false;changed+=1;}
        Ok(())
    }).unwrap();
    assert_eq!(changed,4);
    let mut originals=0;
    iir::visit::expression(&original,&mut 100,|expr,_| {
        if matches!(expr.kind,K::Literal(Literal::Bool(true))) {originals+=1;}
        Ok(())
    }).unwrap();
    assert_eq!(originals,4);
    iir::visit::expression(&expression,&mut 100,|expr,_| {
        assert!(!matches!(expr.kind,K::Literal(Literal::Bool(true))));Ok(())
    }).unwrap();
    assert!(iir::visit::expression_mut(&mut expression,&mut 0,|_,_|Ok(())).is_err());
}

#[test]
fn expression_rewriting_uses_a_bounded_work_stack_on_deep_trees() {
    std::thread::Builder::new().stack_size(2*1024*1024).spawn(|| {
        let mut expression=boolean();
        for _ in 0..4000 {expression=Expr {source:expression.source,ty:expression.ty.clone(),kind:K::Not(Box::new(expression))};}
        let mut visited=0;
        iir::visit::expression_mut(&mut expression,&mut 5000,|_,_| {visited+=1;Ok(())}).unwrap();
        assert_eq!(visited,4001);
        // Iterative destruction keeps the test independent of Box's recursive drop.
        while let K::Not(inner)=expression.kind {expression=*inner;}
    }).unwrap().join().unwrap();
}

#[test]
fn pruned_rewriting_does_not_resubstitute_inserted_caller_references() {
    let original=Expr {kind:K::Local(nir::program::LocalId {owner:1,index:0}),..boolean()};
    let mut expression=original.clone();
    let mut visits=0;
    iir::visit::expression_mut_pruned(&mut expression,&mut 10,|expr,_| {
        visits+=1;
        if matches!(expr.kind,K::Local(_)) {
            expr.kind=K::Not(Box::new(expr.clone()));
            return Ok(false);
        }
        Ok(true)
    }).unwrap();
    assert_eq!(visits,1);
    assert_eq!(expression.kind,K::Not(Box::new(original)));
    assert!(iir::visit::expression_mut_pruned(&mut expression,&mut 0,|_,_|Ok(true)).is_err());
}
