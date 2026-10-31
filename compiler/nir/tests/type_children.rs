use nir::{Parameter, Primitive, Sort, Term};

#[test]
fn borrowed_children_keep_telescope_order_and_stop_on_failure() {
    let signature = Term::Function {
        parameters: vec![
            Parameter {
                sort: Sort::ComptimeValue,
                ty: Term::ClosureContextType,
            },
            Parameter {
                sort: Sort::Runtime,
                ty: Term::Primitive(Primitive::Bool),
            },
        ],
        result: Box::new(Term::Primitive(Primitive::Char)),
    };
    let Term::Function { parameters, result } = &signature else {
        unreachable!()
    };
    let mut children = Vec::new();
    signature
        .visit_children(|child| {
            children.push(child);
            Ok::<_, ()>(())
        })
        .unwrap();
    assert_eq!(children.len(), 3);
    assert!(std::ptr::eq(children[0], &parameters[0].ty));
    assert!(std::ptr::eq(children[1], &parameters[1].ty));
    assert!(std::ptr::eq(children[2], result.as_ref()));
    let mut visits = 0;
    assert_eq!(
        signature.visit_children(|_| {
            visits += 1;
            if visits == 2 { Err("stop") } else { Ok(()) }
        }),
        Err("stop")
    );
    assert_eq!(visits, 2);
}
