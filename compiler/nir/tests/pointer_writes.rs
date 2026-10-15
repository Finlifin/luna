use nir::program::{Expr, ExprKind as K, LocalId, SourceRange, pointer_operation_error};
use nir::{Primitive, Term};

#[test]
fn diverging_pointer_write_preserves_storage_type_and_has_no_normal_result() {
    let source = SourceRange { start: 0, end: 1 };
    let pointer = Expr {
        source,
        ty: Term::Pointer(Box::new(Term::Primitive(Primitive::U8))),
        kind: K::Local(LocalId { owner: 1, index: 1 }),
    };
    for (value_ty, result_ty, valid) in [
        (Primitive::U8, Primitive::Void, true),
        (Primitive::Never, Primitive::Never, true),
        (Primitive::Bool, Primitive::Void, false),
        (Primitive::U8, Primitive::Never, false),
        (Primitive::Never, Primitive::Void, false),
    ] {
        let expr = Expr {
            source,
            ty: Term::Primitive(result_ty),
            kind: K::WritePointer {
                pointer: Box::new(pointer.clone()),
                value: Box::new(Expr {
                    source,
                    ty: Term::Primitive(value_ty),
                    kind: K::Local(LocalId { owner: 1, index: 2 }),
                }),
            },
        };
        assert_eq!(pointer_operation_error(&expr).is_none(), valid);
    }
}
