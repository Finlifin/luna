use nir::{AssociatedTypeOrigin, DeclarationId, Parameter, Primitive, Sort, Term};

#[test]
fn owned_type_children_are_disposed_without_recursive_host_frames() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let id = DeclarationId {
                package: 1,
                local: 1,
            };
            let leaf = || Term::Primitive(Primitive::Bool);
            let mut term = leaf();
            for depth in 0..65536 {
                term = match depth % 18 {
                    0 => Term::Pointer(Box::new(term)),
                    1 => Term::ComptimeFunction(Box::new(term)),
                    2 => Term::Tuple(vec![term]),
                    3 => Term::TupleValue(vec![term]),
                    4 => Term::TypeListValue(vec![term]),
                    5 => Term::TypeListConcat(vec![term]),
                    6 => Term::Record(vec![("field".into(), term)]),
                    7 => Term::RecordValue(vec![("value".into(), term)]),
                    8 => Term::NormalForm {
                        head: id,
                        sort: Sort::Type,
                        arguments: vec![term],
                    },
                    9 => Term::Nominal {
                        declaration: id,
                        arguments: vec![term],
                    },
                    10 => Term::Function {
                        parameters: vec![Parameter {
                            sort: Sort::Runtime,
                            ty: term,
                        }],
                        result: Box::new(leaf()),
                    },
                    11 => Term::Function {
                        parameters: vec![],
                        result: Box::new(term),
                    },
                    12 => Term::Closure {
                        environment: vec![term],
                        signature: Box::new(leaf()),
                    },
                    13 => Term::Closure {
                        environment: vec![],
                        signature: Box::new(term),
                    },
                    14 => Term::AssociatedType {
                        origin: Box::new(AssociatedTypeOrigin {
                            interface: id,
                            member: id,
                        }),
                        arguments: vec![term],
                        subject: Box::new(leaf()),
                    },
                    16 => Term::NominalValue {
                        ty: Box::new(term),
                        variant: None,
                        fields: vec![],
                    },
                    17 => Term::NominalValue {
                        ty: Box::new(leaf()),
                        variant: Some(0),
                        fields: vec![term],
                    },
                    _ => Term::Computation {
                        effects: vec![nir::effects::Operation {
                            declaration: id,
                            arguments: vec![term],
                        }],
                        result: Box::new(leaf()),
                    },
                };
            }
            // Disposal also accepts malformed/rejected IR, so this deliberately
            // tests ownership edges rather than constructing a well-typed tree.
            term.drop_iteratively();
            leaf().drop_iteratively();
        })
        .unwrap()
        .join()
        .unwrap();
}
