//! Expected ordinary list payloads, retaining the registered declaration domain.
pub fn types(package: &nir::program::Package, fields: Vec<nir::Term>) -> nir::Term {
    let head = package
        .normal_forms
        .iter()
        .find(|head| head.role == nir::normal_forms::Role::TypeList)
        .expect("registered list head");
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
