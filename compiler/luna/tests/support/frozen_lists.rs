//! Expected ordinary list payloads, retaining the registered declaration domain.
pub fn types(package: &nir::program::Package, fields: Vec<nir::Term>) -> nir::Term {
    let mut heads = package
        .normal_forms
        .iter()
        .filter(|head| head.role == nir::normal_forms::Role::TypeList);
    let head = heads.next().expect("registered list head");
    assert!(
        heads.next().is_none(),
        "fixture must select its list domain explicitly when multiple heads exist"
    );
    nir::Term::NominalValue {
        ty: Box::new(nir::Term::NormalForm {
            head: head.declaration,
            sort: nir::Sort::Type,
            arguments: vec![nir::Term::Primitive(nir::Primitive::Type)],
        }),
        variant: None,
        fields,
    }
}
