use elaboration::{materialization, ptc::Budget};
use nir::program::{Package, RuntimeFunctionIdentity};
use nir::{DeclarationId, Primitive, Term};
use std::collections::BTreeSet;

#[test]
fn signature_only_runtime_identity_is_not_an_executable_iir_root() {
    let declaration = DeclarationId {
        package: 1,
        local: 1,
    };
    let package = Package {
        runtime_function_identities: vec![RuntimeFunctionIdentity {
            declaration,
            type_parameters: vec![],
            signature: Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(Primitive::Bool)),
            },
        }],
        ..Default::default()
    };
    assert!(materialization::dispatch_instances(&package, Budget::default()).is_ok());
    let error = materialization::instantiate(
        &package,
        &[iir::InstanceKey {
            declaration,
            arguments: vec![],
        }],
        BTreeSet::new(),
        Budget::default(),
    )
    .unwrap_err();
    assert!(error.contains("no executable definition"), "{error}");
}
