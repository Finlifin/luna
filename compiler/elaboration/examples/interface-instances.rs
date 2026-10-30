//! Direct NIR construction counterpart to development/examples/parameterized-interfaces.fl.
use nir::program::{Implementation, Package, Trait};
use nir::{DeclarationId as Id, Primitive, Term};
fn id(local: u32) -> Id {
    Id { package: 0, local }
}
fn main() {
    let mut package = Package::default();
    package.traits.push(Trait {
        comptime_only: false,
        parameter_types: Vec::new(),
        associated: vec![],
        requirements: vec![],
        capabilities: vec![],
        declaration: id(10),
        self_parameter: id(11),
        type_parameters: vec![id(12)],
        name: "Accept".into(),
        parents: vec![],
        methods: vec![],
    });
    for (local, primitive) in [(20, Primitive::Bool), (21, Primitive::Char)] {
        package.implementations.push(Implementation {
            phases: Default::default(),
            associated_initializers: vec![],
            parameter_types: vec![],
            associated: vec![],
            declaration: id(local),
            subject: Term::Primitive(Primitive::Bool),
            interface: Some(id(10)),
            interface_arguments: vec![Term::Primitive(primitive)],
            type_parameters: vec![],
            requirements: vec![],
            capabilities: vec![],
            methods: vec![],
        });
    }
    package.dispatch_instances = elaboration::materialization::dispatch_instances(
        &package,
        elaboration::ptc::Budget::default(),
    )
    .unwrap();
    assert_eq!(package.dispatch_instances.len(), 2);
    assert_ne!(
        package.dispatch_instances[0].interface_instance(),
        package.dispatch_instances[1].interface_instance()
    );
    print!("{package}");
}
