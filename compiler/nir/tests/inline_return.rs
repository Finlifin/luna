use nir::{
    DeclarationId, Primitive, Term,
    inline_return::{self, Boundary},
    program::{Expr, ExprKind as K, LocalId, SourceRange},
};
fn ids() -> Boundary {
    let local = |index| LocalId { owner: 99, index };
    Boundary {
        packet: DeclarationId {
            package: 99,
            local: 1,
        },
        slot: local(1),
        exit: local(2),
        result: local(3),
        unreachable: local(4),
    }
}
fn body() -> Expr {
    Expr {
        ty: Term::Primitive(Primitive::Void),
        source: SourceRange { start: 10, end: 20 },
        kind: K::Unit,
    }
}
#[test]
fn failed_boundary_rolls_back_provenance_and_can_be_retried() {
    let mut sources = vec![];
    let site = body().source;
    let failure =
        inline_return::lower(body(), body().ty, ids(), &mut sources, site, &mut 5).unwrap_err();
    assert!(failure.contains("budget"), "{failure}");
    assert!(sources.is_empty());
    let (result, packet, empty_replacements) =
        inline_return::lower(body(), body().ty, ids(), &mut sources, site, &mut 1000).unwrap();
    assert_eq!(packet.variants.len(), 2);
    assert_eq!(empty_replacements.len(), 1);
    assert!(format!("{}", ids()).starts_with("(inline-return-boundary "));
    assert!(format!("{result}").contains("(loop"));
    for (index, origin) in sources.iter().enumerate() {
        assert!(
            origin
                .original
                .expansion_index()
                .is_none_or(|old| (old as usize) < index)
        );
    }
}
#[test]
fn boundary_rejects_colliding_control_and_value_identities() {
    let mut boundary = ids();
    boundary.result = boundary.exit;
    let failure = inline_return::lower(
        body(),
        body().ty,
        boundary,
        &mut vec![],
        body().source,
        &mut 1000,
    )
    .unwrap_err();
    assert!(failure.contains("distinct local identities"));
}

#[test]
fn internal_return_provenance_is_explicit_in_package_dumps() {
    let mut package = nir::program::Package::default();
    let declaration = ids().packet;
    package.internal_return_packets.insert(declaration);
    assert!(
        package
            .to_string()
            .contains(&format!("(internal-return-packet {declaration})"))
    );
    assert!(
        nir::program::Package::default()
            .internal_return_packets
            .is_empty()
    );
}

#[test]
fn generic_result_packets_bind_free_declaration_types_without_unfolding_heads() {
    use nir::{Sort, program::ExprKind};
    let parameter = DeclarationId {
        package: 3,
        local: 7,
    };
    let head = DeclarationId {
        package: 3,
        local: 8,
    };
    let ty = Term::NormalForm {
        head,
        sort: Sort::Type,
        arguments: vec![Term::Rigid {
            declaration: parameter,
            sort: Sort::Type,
        }],
    };
    assert_eq!(
        ty.rigid_parameters()
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>(),
        vec![(parameter, Sort::Type)]
    );
    let input = Expr {
        ty: ty.clone(),
        source: body().source,
        kind: ExprKind::Local(LocalId { owner: 1, index: 1 }),
    };
    let (result, packet, replacements) = inline_return::lower(
        input,
        ty.clone(),
        ids(),
        &mut vec![],
        body().source,
        &mut 1000,
    )
    .unwrap();
    assert_eq!(result.ty, ty);
    assert_eq!(packet.type_parameters, vec![parameter]);
    assert_eq!(packet.variants[1].fields, vec![ty]);
    assert_eq!(replacements.len(), 1);
}
