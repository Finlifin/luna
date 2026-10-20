//! Validate the language-bound value conversion protocol before synthesizing
//! calls. This does not select an implementation, execute a conversion, or
//! authorize copying a comptime object's representation into runtime memory.
use nir::program::{AssociatedContract, MethodContract, Trait};
use nir::{AssociatedTypeOrigin, DeclarationId, Parameter, Primitive, Sort, Term};

/// Borrow the exact declarations checked in this compilation's package.
pub struct Protocol<'a> {
    pub interface: &'a Trait,
    pub runtime_target: &'a AssociatedContract,
    pub method: &'a MethodContract,
}

impl std::fmt::Display for Protocol<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(splice-protocol (interface {}) (runtime-target {}) (method {}))",
            self.interface.declaration, self.runtime_target.declaration, self.method.declaration
        )
    }
}

/// Validate a driver-selected protocol against this elaboration's declarations.
/// Name lookup must not be used to choose the trait identity.
pub fn protocol(traits: &[Trait], id: DeclarationId) -> Result<Protocol<'_>, String> {
    let mut interfaces = traits.iter().filter(|t| t.declaration == id);
    let interface = interfaces
        .next()
        .ok_or("Splice protocol identity is not a declared trait")?;
    if interfaces.next().is_some() {
        return Err("Splice protocol identity has duplicate declarations".into());
    }
    if !interface.type_parameters.is_empty() {
        return Err("Splice protocol must have no explicit type parameters".into());
    }
    let mut targets = interface
        .associated
        .iter()
        .filter(|a| a.name == "RuntimeTarget");
    let runtime_target = targets
        .next()
        .ok_or("Splice protocol lacks RuntimeTarget")?;
    if targets.next().is_some() || runtime_target.ty != Term::Primitive(Primitive::Type) {
        return Err("Splice protocol requires one Type-valued RuntimeTarget".into());
    }
    let mut methods = interface.methods.iter().filter(|m| m.name == "splice");
    let method = methods
        .next()
        .ok_or("Splice protocol lacks its splice method")?;
    if methods.next().is_some() {
        return Err("Splice protocol has duplicate splice methods".into());
    }
    let subject = Term::Rigid {
        declaration: interface.self_parameter,
        sort: Sort::Type,
    };
    let expected = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::ComptimeValue,
            ty: subject.clone(),
        }],
        result: Box::new(Term::AssociatedType {
            origin: Box::new(AssociatedTypeOrigin {
                interface: id,
                member: runtime_target.declaration,
            }),
            arguments: vec![],
            subject: Box::new(subject),
        }),
    };
    // Exact protocol shape also rejects ComptimeFunction: a staged parameter
    // does not make the conversion method a comptime invocation.
    if method.signature != expected
        || method.by_value_receiver
        || method.by_reference_receiver
        || method.parameter_bindings.len() != 1
        || !method.type_parameters.is_empty()
        || !method.type_arguments.is_empty()
    {
        return Err(
            "Splice protocol requires runtime fn splice(comptime data: Self) -> RuntimeTarget"
                .into(),
        );
    }
    Ok(Protocol {
        interface,
        runtime_target,
        method,
    })
}
